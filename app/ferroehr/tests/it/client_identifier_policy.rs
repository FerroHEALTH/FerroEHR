// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A party binds an OAuth 2.0 client only through FerroEHR's own vocabulary
//! (#6542): an `ELEMENT` named `openehr_access::client_id` holding a
//! `DV_IDENTIFIER`, in any archetype, marks its party as a policy party that
//! only an administrator writes, and only that write binds the client. A
//! `DV_IDENTIFIER` merely typed `client_id` is the deployment's own data.
//! No openEHR spec governs this: our own design.

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
/// whose `details` hold the element `identifier`.
fn actor(rm: &str, archetype: &str, name: &str, identifier: &Value) -> Value {
    json!({
        "_type": rm, "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype }, "rm_version": "1.1.0" },
        "name": text(name),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010", "name": text("details"),
            "items": [identifier] },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": text("name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                    "value": text(name) }] } }]
    })
}

/// The `DV_IDENTIFIER` of `client` under [`ISSUER`], typed `client_id`.
fn identifier(client: &str) -> Value {
    json!({ "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": client, "type": "client_id" })
}

/// The client identifier of `client`: an `ELEMENT` named by the coded marker.
fn coded(client: &str) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0011",
        "name": { "_type": "DV_CODED_TEXT", "value": "client_id",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
                "code_string": "client_id" } },
        "value": identifier(client) })
}

/// The same identifier in an `ELEMENT` of the deployment's own naming.
fn bare(client: &str) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0011", "name": text("client"),
        "value": identifier(client) })
}

/// An identifier of another type, which names no client.
fn other(id: &str) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0011", "name": text("local"),
        "value": { "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": id, "type": "local" } })
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

/// A released-only writer storing a `DV_IDENTIFIER` typed `client_id` in an
/// `ORGANISATION`, a `PERSON` or an `AGENT` of its own archetypes is never
/// refused, and no client binds: the issue's case.
#[tokio::test]
async fn a_bare_client_id_identifier_is_plain_data() {
    let (db, svc) = crate::fixtures::service().await;
    let svc = granting(svc).await;
    for (kind, body) in actors(bare) {
        as_holding(&[], create(&svc, kind, &body))
            .await
            .expect("the deployment's own data commits");
    }
    assert_eq!(bound(&db).await, Vec::new(), "no client binds");
}

/// A caller who is no administrator neither creates nor updates an `ACTOR`
/// into carrying the coded client identifier, and no client binds.
#[tokio::test]
async fn a_writer_is_refused_the_coded_client_identifier() {
    let (db, svc) = crate::fixtures::service().await;
    let svc = granting(svc).await;
    for (kind, body) in actors(coded) {
        refused(
            as_holding(&[], create(&svc, kind, &body)).await,
            "the creation",
        );
    }
    for ((kind, plain), (_, carrying)) in actors(other).into_iter().zip(actors(coded)) {
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

/// The administrator's write of the coded client identifier binds each
/// client, in any archetype.
#[tokio::test]
async fn the_administrator_binds_the_coded_client_identifier() {
    let (db, svc) = crate::fixtures::service().await;
    let svc = granting(svc).await;
    for (kind, body) in actors(coded) {
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
