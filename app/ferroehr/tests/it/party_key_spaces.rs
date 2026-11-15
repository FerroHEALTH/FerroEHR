// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The key spaces of the binding side table (#5688), over a real PostgreSQL
//! 18 (shared testkit harness): a key issuer added while replicas serve is
//! pending until every live replica names it, then filled online to the rows
//! the offline refill builds; a key issuer a replica drops retires at once on
//! every replica and its rows go once no live replica names it; a portal
//! flag change is a retire followed by an add; a duplicate under a new issuer
//! keeps it pending; and a change to the rest of the mapping is still refused
//! at boot. Each replica is a service over the one database with its own
//! mapping and lease. No openEHR spec governs the side table: our own design.

use ferrohealth_smart::party::source::Lookup;
use serde_json::{Value, json};

use crate::fixtures::{if_match, object, written_vo};
use ferroehr::ids::VoId;
use ferroehr::party_binding::fill::{FillError, LEASE_TTL, Lease};
use ferroehr::party_binding::key_space::{KeySpace, KeySpaceState};
use ferroehr::party_binding::lookup::LookupError;
use ferroehr::party_binding::{BindingMapping, IdentifierSelector};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;

const PERSON_TEMPLATE: &str = "FerroEHR professional";
const ROLE_TEMPLATE: &str = "FerroEHR professional role";
const CLUSTER: &str = "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1";
/// The key issuer every replica names.
const LOGIN: &str = "https://login.example/tenant";
/// The key issuer added and removed.
const SECOND: &str = "https://second.example/tenant";
const GUID: &str = "guid";
const NAMESPACE: &str = "demographic";

/// The mapping holding keys under `issuers`, `portal` among them `patients`
/// issuers.
fn mapping(issuers: &[&str], portal: &[&str]) -> BindingMapping {
    let key = IdentifierSelector::new(
        format!("/details[at0001]/items[{CLUSTER}]/items[at0001]"),
        GUID,
    )
    .expect("compiles")
    .with_issuers(issuers.iter().copied());
    BindingMapping::default()
        .with_person(PERSON_TEMPLATE, key)
        .with_professional_role(ROLE_TEMPLATE)
        .with_portal_issuers(portal.iter().copied())
}

/// A replica over `db` with `mapping` installed.
fn replica(db: &testkit::TestDb, mapping: BindingMapping) -> FerroEhrService {
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_binding_mapping(mapping)
}

/// `replica`'s lease, which must be taken.
async fn lease(svc: &FerroEhrService) -> Lease {
    svc.take_binding_lease(uuid::Uuid::now_v7())
        .await
        .expect("the replica boots")
        .expect("a lease")
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn archetyped(archetype: &str, template: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "template_id": { "_type": "TEMPLATE_ID", "value": template },
            "rm_version": "1.1.0" })
}

fn identities() -> Value {
    json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0002", "name": text("name"),
             "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0003",
                          "name": text("structure"),
                          "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0004",
                                      "name": text("name"), "value": text("professional") }] } }])
}

fn party_ref(class: &str, id: VoId) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": NAMESPACE, "type": class,
            "id": { "_type": "HIER_OBJECT_ID", "value": id.to_string() } })
}

/// A `PERSON` of the mapped template carrying each `(issuer, key)`.
fn person(keys: &[(&str, &str)]) -> Value {
    let items: Vec<Value> = keys
        .iter()
        .map(|(issuer, key)| {
            json!({ "_type": "ELEMENT", "archetype_node_id": "at0001", "name": text("key"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": key,
                               "type": GUID } })
        })
        .collect();
    json!({
        "_type": "PERSON", "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-PERSON.person.v1", PERSON_TEMPLATE),
        "name": text("professional"), "identities": identities(),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("details"),
            "items": [{ "_type": "CLUSTER", "archetype_node_id": CLUSTER, "name": text("ids"),
                        "items": items }] }
    })
}

fn version<T: serde::de::DeserializeOwned>(
    body: &Value,
    preceding: Option<&str>,
) -> WriteRequest<T> {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

async fn create(svc: &FerroEhrService, kind: PartyKind, body: &Value) -> (VoId, String) {
    let created = Box::pin(svc.party_create(kind, version(body, None)))
        .await
        .expect("the party commits");
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    (written_vo(&created), uid)
}

async fn update(svc: &FerroEhrService, vo: VoId, uid: &str, body: &Value) -> String {
    let updated = Box::pin(svc.party_update(
        PartyKind::Person,
        object(vo.to_string()),
        if_match(uid),
        version(body, Some(uid)),
    ))
    .await
    .expect("the PERSON updates");
    updated.meta.expect("a version").uid
}

/// A professional carrying `keys`: the `PERSON`, its last version and body.
async fn professional(svc: &FerroEhrService, keys: &[(&str, &str)]) -> (VoId, String, Value) {
    let body = person(keys);
    let (vo, uid) = create(svc, PartyKind::Person, &body).await;
    let role = json!({
        "_type": "ROLE", "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.professional.v1",
        "archetype_details": archetyped("openEHR-DEMOGRAPHIC-ROLE.professional.v1", ROLE_TEMPLATE),
        "name": text("professional"), "identities": identities(),
        "performer": party_ref("PERSON", vo)
    });
    let (role_vo, _) = create(svc, PartyKind::Role, &role).await;
    let mut listed = body;
    listed["roles"] = json!([party_ref("ROLE", role_vo)]);
    let uid = update(svc, vo, &uid, &listed).await;
    (vo, uid, listed)
}

/// What `svc`'s source reads for the key `value` under `issuer`: the bound
/// parties, or the closed key space's state.
async fn bound(svc: &FerroEhrService, issuer: &str, value: &str) -> Result<Vec<String>, String> {
    match svc
        .party_source(NAMESPACE)
        .read(issuer, GUID, value, None, "2026-06-01")
        .await
    {
        Ok(Lookup::Parties(records)) => {
            Ok(records.iter().map(|r| r.party().id().to_owned()).collect())
        }
        Ok(other) => Ok(vec![format!("{other:?}")]),
        Err(LookupError::Closed(closed)) => Err(closed
            .state
            .map_or("not recorded", KeySpaceState::as_str)
            .to_owned()),
        Err(e) => panic!("the source fails: {e}"),
    }
}

/// The stored key spaces.
async fn spaces(db: &testkit::TestDb) -> Vec<KeySpace> {
    replica(db, BindingMapping::default())
        .binding_key_spaces()
        .await
        .expect("the key spaces")
}

fn space(issuer: &str, portal: bool, state: KeySpaceState) -> KeySpace {
    KeySpace {
        issuer: issuer.to_owned(),
        portal,
        state,
    }
}

/// Every key row under `issuer`, live and retired, without its timestamps.
async fn key_rows(db: &testkit::TestDb, issuer: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT party_id::text || ' ' || identifier_type || ' ' || value || ' ' || \
                lifecycle_state || ' ' || professional::text || ' ' || representative::text || \
                ' ' || (retired_at IS NULL)::text || ' ' || (deprovisioned_at IS NULL)::text \
         FROM party.binding_identifier WHERE selector = 'key' AND issuer = $1 \
           AND (retired_at IS NULL OR deprovisioned_at IS NOT NULL) ORDER BY 1",
    )
    .bind(issuer)
    .fetch_all(&db.pool())
    .await
    .expect("the key rows")
}

#[tokio::test]
async fn an_added_issuer_is_pending_until_every_replica_names_it_then_fills_online() {
    let (db, _) = crate::fixtures::service().await;
    let old = replica(&db, mapping(&[LOGIN], &[]));
    let first = lease(&old).await;
    let (login, second) = (
        "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d",
        "2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e",
    );
    let (vo, uid, body) = professional(&old, &[(LOGIN, login), (SECOND, second)]).await;
    let deleted = "3c4d5e6f-7a8b-4c9d-8e0f-2a3b4c5d6e7f";
    let (_, gone_uid, _) = professional(&old, &[(SECOND, deleted)]).await;
    Box::pin(old.party_delete(
        PartyKind::Person,
        object(&gone_uid),
        crate::fixtures::delete_precondition(&gone_uid, None),
        WriteRequest::new(()),
    ))
    .await
    .expect("the PERSON is deleted");

    // A replica naming the second issuer boots beside the old one and
    // records it pending: nobody binds under it, the first issuer binds on
    // both.
    let new = replica(&db, mapping(&[LOGIN, SECOND], &[]));
    let second_lease = lease(&new).await;
    assert_eq!(
        spaces(&db).await,
        [
            space(LOGIN, false, KeySpaceState::Active),
            space(SECOND, false, KeySpaceState::Pending)
        ]
    );
    assert_eq!(bound(&new, SECOND, second).await, Err("pending".to_owned()));
    assert_eq!(bound(&old, SECOND, second).await, Err("pending".to_owned()));
    assert_eq!(bound(&new, LOGIN, login).await, Ok(vec![vo.to_string()]));
    assert_eq!(bound(&old, LOGIN, login).await, Ok(vec![vo.to_string()]));

    // The new replica tends while the old one still runs: nothing fills.
    second_lease.tend().await.expect("tended");
    assert_eq!(bound(&new, SECOND, second).await, Err("pending".to_owned()));
    // A commit on the old replica serves on.
    update(&old, vo, &uid, &body).await;

    // The old replica is replaced by one naming the issuer: the next tend
    // fills it online and it binds on both.
    first.release().await.expect("released");
    let replaced = replica(&db, mapping(&[LOGIN, SECOND], &[]));
    let third = lease(&replaced).await;
    second_lease.tend().await.expect("tended");
    assert_eq!(
        spaces(&db).await,
        [
            space(LOGIN, false, KeySpaceState::Active),
            space(SECOND, false, KeySpaceState::Active)
        ]
    );
    assert_eq!(bound(&new, SECOND, second).await, Ok(vec![vo.to_string()]));
    assert_eq!(
        bound(&replaced, SECOND, second).await,
        Ok(vec![vo.to_string()])
    );
    assert_eq!(
        bound(&replaced, SECOND, deleted).await,
        Ok(vec!["Deprovisioned".to_owned()]),
        "a deleted PERSON's key under the new issuer is a tombstone"
    );

    // The online fill built the rows the offline refill builds.
    let online = key_rows(&db, SECOND).await;
    assert_eq!(online.len(), 2, "{online:?}");
    second_lease.release().await.expect("released");
    third.release().await.expect("released");
    replaced.refill_binding(LEASE_TTL).await.expect("refilled");
    assert_eq!(key_rows(&db, SECOND).await, online);
}

#[tokio::test]
async fn a_removed_issuer_retires_at_once_and_goes_once_no_replica_names_it() {
    let (db, _) = crate::fixtures::service().await;
    let both = mapping(&[LOGIN, SECOND], &[]);
    let one = replica(&db, both.clone());
    let first = lease(&one).await;
    let two = replica(&db, both);
    let second = lease(&two).await;
    let (login, key) = (
        "4d5e6f7a-8b9c-4d0e-9f1a-3b4c5d6e7f80",
        "5e6f7a8b-9c0d-4e1f-8a2b-4c5d6e7f8091",
    );
    let (vo, ..) = professional(&one, &[(LOGIN, login), (SECOND, key)]).await;
    assert_eq!(bound(&two, SECOND, key).await, Ok(vec![vo.to_string()]));

    // The first replica restarts without the issuer: it stops binding on the
    // other replica at once, with no restart of it.
    first.release().await.expect("released");
    let dropped = replica(&db, mapping(&[LOGIN], &[]));
    let third = lease(&dropped).await;
    assert_eq!(bound(&two, SECOND, key).await, Err("retiring".to_owned()));
    assert_eq!(
        bound(&dropped, SECOND, key).await,
        Err("retiring".to_owned())
    );
    assert_eq!(bound(&two, LOGIN, login).await, Ok(vec![vo.to_string()]));

    // Its rows stay while a live replica names it.
    third.tend().await.expect("tended");
    second.tend().await.expect("tended");
    assert_eq!(key_rows(&db, SECOND).await.len(), 1);
    assert_eq!(bound(&two, SECOND, key).await, Err("retiring".to_owned()));

    // Once no live replica names it, the key space and its rows go.
    second.release().await.expect("released");
    third.tend().await.expect("tended");
    assert_eq!(
        spaces(&db).await,
        [space(LOGIN, false, KeySpaceState::Active)]
    );
    assert_eq!(key_rows(&db, SECOND).await, Vec::<String>::new());
    assert_eq!(
        bound(&dropped, SECOND, key).await,
        Err("not recorded".to_owned()),
        "an issuer no key space records binds nothing"
    );
}

#[tokio::test]
async fn a_portal_flag_change_retires_then_adds_the_issuer() {
    let (db, _) = crate::fixtures::service().await;
    let plain = replica(&db, mapping(&[LOGIN, SECOND], &[]));
    let first = lease(&plain).await;
    let key = "6f7a8b9c-0d1e-4f2a-9b3c-5d6e7f8091a2";
    let (vo, ..) = professional(&plain, &[(SECOND, key)]).await;
    first.release().await.expect("released");

    let portal = replica(&db, mapping(&[LOGIN, SECOND], &[SECOND]));
    let second = lease(&portal).await;
    assert_eq!(
        spaces(&db).await,
        [
            space(LOGIN, false, KeySpaceState::Active),
            space(SECOND, false, KeySpaceState::Retiring)
        ]
    );
    assert_eq!(
        bound(&portal, SECOND, key).await,
        Err("retiring".to_owned())
    );
    second.tend().await.expect("tended");
    assert_eq!(
        spaces(&db).await,
        [
            space(LOGIN, false, KeySpaceState::Active),
            space(SECOND, true, KeySpaceState::Active)
        ]
    );
    assert_eq!(bound(&portal, SECOND, key).await, Ok(vec![vo.to_string()]));
}

#[tokio::test]
async fn a_duplicate_under_an_added_issuer_keeps_it_pending_naming_both_parties() {
    let (db, _) = crate::fixtures::service().await;
    let old = replica(&db, mapping(&[LOGIN], &[]));
    let first = lease(&old).await;
    let key = "7a8b9c0d-1e2f-4a3b-8c4d-6e7f8091a2b3";
    let (one, ..) = professional(&old, &[(SECOND, key)]).await;
    let (two, ..) = professional(&old, &[(SECOND, key)]).await;
    first.release().await.expect("released");

    let new = replica(&db, mapping(&[LOGIN, SECOND], &[]));
    let second = lease(&new).await;
    let refused = second.tend().await.expect_err("a key carried twice");
    let FillError::Duplicate { duplicates } = &refused else {
        panic!("{refused:?}");
    };
    assert_eq!(duplicates.len(), 1, "{duplicates:?}");
    let text = refused.to_string();
    assert!(
        text.contains(&one.to_string()) && text.contains(&two.to_string()),
        "{text}"
    );
    assert_eq!(
        spaces(&db).await,
        [
            space(LOGIN, false, KeySpaceState::Active),
            space(SECOND, false, KeySpaceState::Pending)
        ]
    );
    assert_eq!(bound(&new, SECOND, key).await, Err("pending".to_owned()));
}

#[tokio::test]
async fn a_rerun_fill_converges_and_a_commit_during_it_keeps_its_head() {
    let (db, _) = crate::fixtures::service().await;
    let svc = replica(&db, mapping(&[LOGIN, SECOND], &[]));
    let held = lease(&svc).await;
    let before = "8b9c0d1e-2f3a-4b4c-9d5e-7f8091a2b3c4";
    let (_, uid, _) = professional(&svc, &[(SECOND, before)]).await;
    let rows = key_rows(&db, SECOND).await;

    // A fill run again from the start leaves the rows it found.
    sqlx::query("UPDATE party.binding_key_space SET state = 'filling' WHERE issuer = $1")
        .bind(SECOND)
        .execute(&db.pool())
        .await
        .expect("the fill restarts");
    held.tend().await.expect("tended");
    assert_eq!(key_rows(&db, SECOND).await, rows);

    // A commit while the key space fills writes its own key rows, and the
    // fill keeps what the head holds: a deleted PERSON's key is a tombstone.
    sqlx::query("UPDATE party.binding_key_space SET state = 'filling' WHERE issuer = $1")
        .bind(SECOND)
        .execute(&db.pool())
        .await
        .expect("the fill restarts");
    Box::pin(svc.party_delete(
        PartyKind::Person,
        object(&uid),
        crate::fixtures::delete_precondition(&uid, None),
        WriteRequest::new(()),
    ))
    .await
    .expect("the PERSON is deleted");
    held.tend().await.expect("tended");
    assert_eq!(
        bound(&svc, SECOND, before).await,
        Ok(vec!["Deprovisioned".to_owned()])
    );
    held.release().await.expect("released");
}

#[tokio::test]
async fn a_change_to_the_rest_of_the_mapping_is_still_refused_at_boot() {
    let (db, _) = crate::fixtures::service().await;
    let first = lease(&replica(&db, mapping(&[LOGIN], &[]))).await;
    let changed = mapping(&[LOGIN, SECOND], &[]).with_professional_role("Another role");
    let refused = replica(&db, changed)
        .take_binding_lease(uuid::Uuid::now_v7())
        .await
        .expect_err("another role refuses to start");
    assert!(matches!(refused, FillError::Digest { .. }), "{refused:?}");
    assert!(
        refused.to_string().contains("party-binding refill"),
        "{refused}"
    );
    first.release().await.expect("released");
}

#[tokio::test]
async fn a_retiring_issuer_every_replica_names_again_fills_anew() {
    let (db, _) = crate::fixtures::service().await;
    let both = mapping(&[LOGIN, SECOND], &[]);
    let stays = replica(&db, both.clone());
    let kept = lease(&stays).await;
    let key = "a0b1c2d3-e4f5-4a6b-9c7d-8e9f0a1b2c3d";
    let (vo, uid, listed) = professional(&stays, &[(SECOND, key)]).await;

    // A replica runs without the issuer and commits a new key meanwhile, so
    // the rows under the issuer no longer follow the head.
    let dropped = replica(&db, mapping(&[LOGIN], &[]));
    let without = lease(&dropped).await;
    let moved_key = "b1c2d3e4-f5a6-4b7c-8d9e-0f1a2b3c4d5e";
    let mut moved = person(&[(SECOND, moved_key)]);
    moved["roles"] = listed["roles"].clone();
    update(&dropped, vo, &uid, &moved).await;
    assert_eq!(bound(&stays, SECOND, key).await, Err("retiring".to_owned()));

    // It restarts naming the issuer again: the key space fills anew and
    // binds the key the head holds.
    without.release().await.expect("released");
    let back = replica(&db, both);
    let returned = lease(&back).await;
    kept.tend().await.expect("tended");
    assert_eq!(
        spaces(&db).await,
        [
            space(LOGIN, false, KeySpaceState::Active),
            space(SECOND, false, KeySpaceState::Active)
        ]
    );
    assert_eq!(
        bound(&back, SECOND, moved_key).await,
        Ok(vec![vo.to_string()])
    );
    assert_eq!(bound(&stays, SECOND, key).await, Ok(Vec::new()));
    returned.release().await.expect("released");
    kept.release().await.expect("released");
}
