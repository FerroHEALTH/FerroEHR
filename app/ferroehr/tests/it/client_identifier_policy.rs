// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `DV_IDENTIFIER` typed `client_id` marks its party and binds its client
//! only under FerroEHR's configured policy (#6542): with the policy, any
//! `ACTOR` of any archetype carrying one is a policy party only an
//! administrator writes, and only that write binds the client; without it
//! the identifier is plain data. No openEHR spec governs this: our own
//! design.

use serde_json::{Value, json};

use crate::fixtures::{if_match, object, written_vo};
use crate::policy_fixtures::{as_admin, as_holding, granting};
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;

const ISSUER: &str = "https://login.example.org/machines";

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// An `ACTOR` of `rm` and the deployment's own `archetype`, named `name`,
/// whose `details` hold the identifier `identifier`.
fn actor(rm: &str, archetype: &str, name: &str, identifier: &Value) -> Value {
    json!({
        "_type": rm, "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype }, "rm_version": "1.1.0" },
        "name": text(name),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0011", "name": text("identifier"),
                "value": identifier }] },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": text("name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                    "value": text(name) }] } }]
    })
}

/// A client identifier of `client` under [`ISSUER`].
fn client(client: &str) -> Value {
    json!({ "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": client, "type": "client_id" })
}

/// An identifier of another type, which names no client.
fn other(id: &str) -> Value {
    json!({ "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": id, "type": "local" })
}

/// The released-only `ACTOR`s a deployment writes, each of its kind, carrying
/// the identifier `identifier`.
fn actors(identifier: impl Fn(&str) -> Value) -> [(PartyKind, Value); 3] {
    [
        (
            PartyKind::Organisation,
            actor(
                "ORGANISATION",
                "openEHR-DEMOGRAPHIC-ORGANISATION.partner.v1",
                "Partner",
                &identifier("partner"),
            ),
        ),
        (
            PartyKind::Person,
            actor(
                "PERSON",
                "openEHR-DEMOGRAPHIC-PERSON.person.v2",
                "Jan Jansen",
                &identifier("person"),
            ),
        ),
        (
            PartyKind::Agent,
            actor(
                "AGENT",
                "openEHR-DEMOGRAPHIC-AGENT.lab_system.v1",
                "Lab system",
                &identifier("agent"),
            ),
        ),
    ]
}

fn write<T: serde::de::DeserializeOwned>(body: &Value, preceding: Option<&str>) -> WriteRequest<T> {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

async fn create(
    svc: &FerroEhrService,
    kind: PartyKind,
    body: &Value,
) -> Result<(VoId, String), ServiceError> {
    let created = Box::pin(svc.party_create(kind, write(body, None))).await?;
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    Ok((written_vo(&created), uid))
}

async fn update(
    svc: &FerroEhrService,
    kind: PartyKind,
    (vo, uid): (VoId, &str),
    body: &Value,
) -> Result<(), ServiceError> {
    Box::pin(svc.party_update(
        kind,
        object(vo.to_string()),
        if_match(uid),
        write(body, Some(uid)),
    ))
    .await
    .map(drop)
}

/// The client rows of `db`, as `(issuer, client_id)`, sorted.
async fn bound(db: &testkit::TestDb) -> Vec<(String, String)> {
    sqlx::query_as("SELECT issuer, client_id FROM party.binding_client ORDER BY issuer, client_id")
        .fetch_all(&db.pool())
        .await
        .expect("the client rows read")
}

fn refused(result: Result<impl std::fmt::Debug, ServiceError>, what: &str) {
    match result.map_err(ServiceError::flattened) {
        Err(ServiceError::Forbidden(_)) => {}
        other => panic!("{what}: expected 403, got {other:?}"),
    }
}

/// With the policy configured, a caller who is no administrator neither
/// creates nor updates an `ORGANISATION`, a `PERSON` or an `AGENT` of the
/// deployment's own archetype into carrying a client identifier, and no
/// client binds.
#[tokio::test]
async fn under_the_policy_a_writer_binds_no_client() {
    let (db, svc) = crate::fixtures::service().await;
    let svc = granting(svc).await;
    for (kind, body) in actors(client) {
        refused(
            as_holding(&[], create(&svc, kind, &body)).await,
            "the creation",
        );
    }
    for ((kind, plain), (_, carrying)) in actors(other).into_iter().zip(actors(client)) {
        let (vo, uid) = as_holding(&[], create(&svc, kind, &plain))
            .await
            .expect("an identifier of another type is plain data");
        refused(
            as_holding(&[], update(&svc, kind, (vo, &uid), &carrying)).await,
            "the update",
        );
    }
    assert_eq!(bound(&db).await, Vec::new(), "no client binds");
}

/// With the policy configured, the administrator's write of the same
/// `ACTOR`s binds each client.
#[tokio::test]
async fn under_the_policy_the_administrator_binds_the_client() {
    let (db, svc) = crate::fixtures::service().await;
    let svc = granting(svc).await;
    for (kind, body) in actors(client) {
        as_admin(create(&svc, kind, &body))
            .await
            .expect("the administrator writes it");
    }
    let expected: Vec<(String, String)> = ["agent", "partner", "person"]
        .into_iter()
        .map(|id| (ISSUER.to_owned(), id.to_owned()))
        .collect();
    assert_eq!(bound(&db).await, expected);
}

/// Without the policy a client identifier is plain data: any writer commits
/// it, and no client binds, not even from the server's own write.
#[tokio::test]
async fn without_the_policy_no_client_binds() {
    let (db, svc) = crate::fixtures::service().await;
    for (kind, body) in actors(client) {
        as_holding(&[], create(&svc, kind, &body))
            .await
            .expect("a client identifier is plain data");
    }
    let (kind, body) = (
        PartyKind::Agent,
        actor(
            "AGENT",
            "openEHR-DEMOGRAPHIC-AGENT.gateway.v1",
            "Gateway",
            &client("gateway"),
        ),
    );
    create(&svc, kind, &body)
        .await
        .expect("the server writes it");
    assert_eq!(bound(&db).await, Vec::new(), "no client binds");
}
