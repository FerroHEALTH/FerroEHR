// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Each replica's copy of the binding lookups and the bound `PERSON`s' names
//! (#5771), over a real PostgreSQL 18 with two services on one store: a held
//! lookup reads nothing, a deprovisioning, a binding change and a rename hold
//! on the next request of every replica once its notification arrives, a
//! request whose commit changed the side table marks it for its own replica,
//! and with the listener down every request reads the store. No openEHR spec
//! governs resolving a login to a party: our own design.

use std::sync::Arc;
use std::time::Duration;

use ferrohealth_smart::party::PartyRef;
use ferrohealth_smart::party::source::Lookup;

use crate::party_binding_source::{
    GUID, LOGIN, NAMESPACE, PERSON_TEMPLATE, TODAY, delete, identities, key, mapping, person,
    professional, update,
};
use ferroehr::db::domain::DomainPools;
use ferroehr::ids::VoId;
use ferroehr::party_binding::live::tracking;
use ferroehr::party_binding::{IdentifierSelector, NameSelector, PersonSelectors};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;

/// How long a replica may take to hear a notification.
const HEARD: Duration = Duration::from_secs(5);

/// The mapping of the source tests, naming each `PERSON` by its identity.
fn named() -> ferroehr::party_binding::BindingMapping {
    let key = IdentifierSelector::new(
        "/details[at0001]/items[openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v1]/items[at0001]",
        GUID,
    )
    .expect("the key selector compiles");
    let name = NameSelector::new("/identities[at0002]/details[at0003]/items[at0004]")
        .expect("the name selector compiles");
    mapping().with_person_selectors(PERSON_TEMPLATE, PersonSelectors::new(key).with_name(name))
}

/// Two replicas over one fresh store, each listening on its own connection.
async fn replicas() -> (testkit::TestDb, FerroEhrService, FerroEhrService) {
    let (db, first) = crate::fixtures::service().await;
    crate::fixtures::open_key_spaces(&db, &[(LOGIN, false)]).await;
    let first = first.with_binding_mapping(named());
    let second =
        FerroEhrService::new(&DomainPools::from_shared(&db.pool())).with_binding_mapping(named());
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
    (db, first, second)
}

/// What `replica` holds for the login key `id`.
async fn lookup(replica: &FerroEhrService, id: &str) -> Lookup {
    replica
        .party_source(NAMESPACE)
        .read(LOGIN, GUID, id, None, TODAY)
        .await
        .expect("the source answers")
}

/// The parties a lookup names.
fn parties(lookup: &Lookup) -> Vec<PartyRef> {
    match lookup {
        Lookup::Parties(records) => records.iter().map(|r| r.party().clone()).collect(),
        other => panic!("expected parties, got {other:?}"),
    }
}

fn person_ref(vo: VoId) -> PartyRef {
    PartyRef::new(NAMESPACE, "PERSON", vo.to_string())
}

/// Waits until `replica` has heard a notification after the `seen`th.
async fn heard(replica: &FerroEhrService, seen: u64) {
    assert!(
        replica
            .bindings()
            .listened()
            .notified_after(seen, HEARD)
            .await,
        "the replica hears the change"
    );
}

/// Waits until `replica` has heard every notification of the commits before,
/// so a lookup held after it is dropped by none of them.
async fn settled(replica: &FerroEhrService) {
    let listened = replica.bindings().listened();
    loop {
        let seen = listened.notifications();
        if !listened
            .notified_after(seen, Duration::from_millis(300))
            .await
        {
            return;
        }
    }
}

/// The side table changed under the triggers' backs, as only its owner can:
/// no replica hears of it.
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn unheard(db: &testkit::TestDb, statement: &str) {
    let pool = db.pool();
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("ALTER TABLE party.binding_identifier DISABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("disable the triggers");
    sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
        .execute(&mut *tx)
        .await
        .expect("the change applies");
    sqlx::query("ALTER TABLE party.binding_identifier ENABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("enable the triggers");
    tx.commit().await.expect("commit");
}

/// A held lookup is answered without a read, and a deprovisioning on one
/// replica holds on the other's next lookup once it hears it.
#[tokio::test]
async fn a_deprovisioning_holds_on_every_replica() {
    let (db, first, second) = replicas().await;
    let value = "1b2c3d4e-5f60-4a71-8b92-a3b4c5d6e7f8";
    let (vo, uid) = professional(&first, value).await;
    settled(&second).await;
    assert_eq!(parties(&lookup(&second, value).await), [person_ref(vo)]);

    unheard(
        &db,
        "UPDATE party.binding_identifier SET value = 'elsewhere' WHERE selector = 'key'",
    )
    .await;
    assert_eq!(
        parties(&lookup(&second, value).await),
        [person_ref(vo)],
        "a held lookup reads no party-store statement"
    );
    unheard(
        &db,
        &format!("UPDATE party.binding_identifier SET value = '{value}' WHERE selector = 'key'"),
    )
    .await;

    let seen = second.bindings().listened().notifications();
    let ((), changed) = tracking(delete(&first, PartyKind::Person, &uid)).await;
    assert!(
        changed,
        "the deprovisioning commit marks the side table changed"
    );
    first.bindings().forget_all();
    assert_eq!(
        lookup(&first, value).await,
        Lookup::Deprovisioned,
        "the replica that committed drops its copy before it answers"
    );
    heard(&second, seen).await;
    assert_eq!(lookup(&second, value).await, Lookup::Deprovisioned);
}

/// A new binding holds on the next lookup of a replica that held the key
/// unbound.
#[tokio::test]
async fn a_new_binding_holds_on_every_replica() {
    let (_db, first, second) = replicas().await;
    let value = "2c3d4e5f-6071-4b82-9ca3-b4c5d6e7f809";
    assert_eq!(parties(&lookup(&second, value).await), []);
    let seen = second.bindings().listened().notifications();
    let (vo, _) = professional(&first, value).await;
    heard(&second, seen).await;
    assert_eq!(parties(&lookup(&second, value).await), [person_ref(vo)]);
}

/// A rename holds on the next name read of every replica.
#[tokio::test]
async fn a_rename_holds_on_every_replica() {
    let (_db, first, second) = replicas().await;
    let value = "3d4e5f60-7182-4c93-8db4-c5d6e7f8091a";
    let (vo, uid) = professional(&first, value).await;
    let name = |replica: &FerroEhrService| {
        let source = replica.party_source(NAMESPACE);
        async move { source.name(vo.0).await.expect("the name") }
    };
    assert_eq!(name(&second).await.as_deref(), Some("professional"));

    let mut renamed = person(PERSON_TEMPLATE, &[key(value)]);
    renamed["identities"] = identities("renamed");
    let seen = second.bindings().listened().notifications();
    update(&first, PartyKind::Person, vo, &uid, &renamed, "532").await;
    heard(&second, seen).await;
    assert_eq!(name(&second).await.as_deref(), Some("renamed"));
}

/// With the listener down, every lookup reads the store.
#[tokio::test]
async fn with_the_listener_down_every_lookup_reads_the_store() {
    let (db, svc) = crate::fixtures::service().await;
    crate::fixtures::open_key_spaces(&db, &[(LOGIN, false)]).await;
    let svc = svc.with_binding_mapping(named());
    assert!(!svc.bindings().listened().is_listening());
    let value = "4e5f6071-8293-4da4-9ec5-d6e7f8091a2b";
    let (vo, _) = professional(&svc, value).await;
    assert_eq!(parties(&lookup(&svc, value).await), [person_ref(vo)]);
    unheard(
        &db,
        "UPDATE party.binding_identifier SET value = 'elsewhere' WHERE selector = 'key'",
    )
    .await;
    assert_eq!(
        parties(&lookup(&svc, value).await),
        [],
        "the lookup read the store again"
    );
}
