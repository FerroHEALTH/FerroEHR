// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Policy parties for the tests that read a grant (#5439): a deployment's own
//! `AGENT` carrying a client identifier and a `ROLE` whose capability codes an
//! operation of `ferroehr-operations` with its scope elements, committed by an
//! administrator, and the request scope that carries the client's grants. No
//! openEHR spec gives a capability an authorization meaning: our own design.

use std::future::Future;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::fixtures::written_vo;
use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::capabilities::{Actor, Capabilities, with_capabilities};
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

/// The token issuer of the system clients of these tests.
pub(crate) const ISSUER: &str = "https://login.example.org/machines";
/// The administrator role of these tests.
const ADMIN: &str = "ADMIN";

/// `svc` holding the policy definitions under an empty party mapping, as the
/// server installs it, honouring every operation.
pub(crate) async fn granting(svc: FerroEhrService) -> FerroEhrService {
    svc.seed_default_definitions()
        .await
        .expect("the policy definitions are seeded");
    let config = ferroehr::config::FerroEhrConfig::default();
    let sections = config.authz.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[], ADMIN)
        .expect("the mapping compiles");
    svc.with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping)
        .with_grant_ceiling(GrantCeiling::new(Operation::ALL))
}

/// Runs `fut` as an administrator.
pub(crate) async fn as_admin<F: Future>(fut: F) -> F::Output {
    let caller = CommitterIdentity {
        subject: "admin".to_owned(),
        id_type: "oauth2",
        issuer: Some("https://login.example".to_owned()),
        client: None,
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: false,
        system_client: None,
        external: None,
    };
    ferroehr::system_log::access_context::with_roles(
        vec![ADMIN.to_owned()],
        with_committer(Some(caller), Box::pin(fut)),
    )
    .await
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// A `DV_CODED_TEXT` of the operation terminology.
pub(crate) fn operation_code(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

fn archetyped(archetype: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" })
}

fn identities(name: &str) -> Value {
    json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": text("name"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                        "value": text(name) }] } }])
}

/// A credentials element whose name is the terminology's `code`, holding the
/// `DV_TEXT` `value`.
pub(crate) fn scope(code: &str, value: &str) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0030", "name": operation_code(code),
            "value": text(value) })
}

/// A credentials element whose name is the terminology's `code`, holding a
/// `DV_IDENTIFIER` of `id`.
pub(crate) fn identified(code: &str, id: &str) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0031", "name": operation_code(code),
            "value": { "_type": "DV_IDENTIFIER", "issuer": "urn:example", "id": id,
                       "type": "system" } })
}

/// A capability granting `operation`, with the further credentials `items`.
pub(crate) fn capability(operation: &str, items: &[Value]) -> Value {
    let mut all = vec![json!({ "_type": "ELEMENT", "archetype_node_id": "at0021",
                               "name": text("operation"), "value": operation_code(operation) })];
    all.extend_from_slice(items);
    json!({
        "_type": "CAPABILITY", "archetype_node_id": "at0020", "name": text("grant"),
        "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0022", "name": text("terms"),
                         "items": all }
    })
}

/// An `AGENT` of the deployment's own archetype carrying the client
/// identifier `client_id` of [`ISSUER`].
fn agent(client_id: &str) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-AGENT.connected_system.v1";
    json!({
        "_type": "AGENT", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Connected system"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0011", "name": text("client"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": client_id,
                           "type": "client_id" } }] },
        "identities": identities(client_id)
    })
}

/// A `ROLE` of the deployment's own archetype performed by `performer`,
/// holding `capabilities`.
pub(crate) fn role(performer: VoId, capabilities: &[Value]) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.connected_system.v1";
    json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Connected system"),
        "identities": identities("connected system"),
        "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
                       "id": { "_type": "HIER_OBJECT_ID", "value": performer.to_string() } },
        "capabilities": capabilities
    })
}

/// Creates `body` as a party of `kind` as an administrator; its container
/// and version uid.
///
/// # Errors
/// The refusal of the commit.
pub(crate) async fn create(
    svc: &FerroEhrService,
    kind: PartyKind,
    body: &Value,
) -> Result<(VoId, String), ferroehr::error::ServiceError> {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    let created = as_admin(async { Box::pin(svc.party_create(kind, write)).await }).await?;
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    Ok((written_vo(&created), uid))
}

/// Deletes the party of `kind` at its version `uid` as an administrator.
pub(crate) async fn delete(svc: &FerroEhrService, kind: PartyKind, uid: &str) {
    as_admin(async {
        Box::pin(svc.party_delete(
            kind,
            crate::fixtures::object(uid),
            crate::fixtures::delete_precondition(uid, None),
            WriteRequest::new(()),
        ))
        .await
    })
    .await
    .expect("the party is deleted");
}

/// Commits the `AGENT` of `client_id` and a `ROLE` it performs holding
/// `capabilities`; the `AGENT`'s container and the `ROLE`'s version uid.
pub(crate) async fn granted(
    svc: &FerroEhrService,
    client_id: &str,
    capabilities: &[Value],
) -> (VoId, String) {
    let (agent, _) = create(svc, PartyKind::Agent, &agent(client_id))
        .await
        .expect("the AGENT commits");
    let (_, role_uid) = create(svc, PartyKind::Role, &role(agent, capabilities))
        .await
        .expect("the ROLE commits");
    (agent, role_uid)
}

/// The grants of the client `client_id` on `svc`, after it read the grant
/// rows.
pub(crate) async fn of_client(svc: &FerroEhrService, client_id: &str) -> Arc<Capabilities> {
    svc.grants().refresh().await.expect("the grants read");
    Arc::new(svc.grants().capabilities(
        Actor::Client {
            issuer: ISSUER,
            client_id,
        },
        jiff::Timestamp::now(),
    ))
}

/// Runs `fut` as a request of the client `client_id` on `svc`, inside its
/// grants.
pub(crate) async fn as_client<F: Future>(svc: &FerroEhrService, client_id: &str, fut: F) -> F::Output {
    with_capabilities(of_client(svc, client_id).await, fut).await
}
