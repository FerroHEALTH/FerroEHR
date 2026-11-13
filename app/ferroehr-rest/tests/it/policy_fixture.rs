// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Policy parties for the wire tests that read a grant (#5439): a
//! deployment's own `AGENT` carrying a client identifier and a `ROLE` whose
//! capability codes an operation of `ferroehr-operations` with its scope
//! elements, committed through the service by an administrator. No openEHR
//! spec gives a capability an authorization meaning: our own design.

use std::sync::Arc;

use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};
use serde_json::{Value, json};

/// The administrator role of these tests.
const ADMIN: &str = "ADMIN";

/// A service over a migrated database holding the policy definitions under
/// an empty party mapping, as the server installs it, honouring every
/// operation.
pub(crate) async fn granting_service() -> (testkit::TestDb, Arc<FerroEhrService>) {
    let (db, pool) = crate::common::migrated_pool().await;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    svc.seed_default_definitions()
        .await
        .expect("the policy definitions are seeded");
    let config = ferroehr::config::FerroEhrConfig::default();
    let sections = config.authz.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[], ADMIN)
        .expect("the mapping compiles");
    let svc = svc
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping)
        .with_grant_ceiling(GrantCeiling::new(Operation::ALL));
    (db, Arc::new(svc))
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn coded(code: &str) -> Value {
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
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0030", "name": coded(code),
            "value": text(value) })
}

/// A capability granting `operation`, with the further credentials `items`.
pub(crate) fn capability(operation: &str, items: &[Value]) -> Value {
    let mut all = vec![json!({ "_type": "ELEMENT", "archetype_node_id": "at0021",
                               "name": text("operation"), "value": coded(operation) })];
    all.extend_from_slice(items);
    json!({
        "_type": "CAPABILITY", "archetype_node_id": "at0020", "name": text("grant"),
        "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0022", "name": text("terms"),
                         "items": all }
    })
}

/// Creates `body` as a party of `kind` on `svc` as an administrator; its
/// container.
async fn create(svc: &FerroEhrService, kind: PartyKind, body: &Value) -> VoId {
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
    let mut write = WriteRequest::new(
        openehr_its::json::from_canonical_value(body).expect("a canonical party"),
    );
    write.lifecycle_state = Some("532".to_owned());
    let created = ferroehr::system_log::access_context::with_roles(
        vec![ADMIN.to_owned()],
        with_committer(Some(caller), Box::pin(svc.party_create(kind, write))),
    )
    .await
    .expect("the policy party commits");
    let uid = created.meta.expect("a version").uid;
    VoId(
        uid.split_once("::")
            .map_or(uid.as_str(), |(vo, _)| vo)
            .parse()
            .expect("a container uuid"),
    )
}

/// Commits the `AGENT` of `client_id` at `issuer` and a `ROLE` it performs
/// holding `capabilities` on `svc`; the `AGENT`'s container.
pub(crate) async fn grant(
    svc: &FerroEhrService,
    issuer: &str,
    client_id: &str,
    capabilities: &[Value],
) -> VoId {
    let agent_archetype = "openEHR-DEMOGRAPHIC-AGENT.connected_system.v1";
    let agent = create(
        svc,
        PartyKind::Agent,
        &json!({
            "_type": "AGENT", "archetype_node_id": agent_archetype,
            "archetype_details": archetyped(agent_archetype), "name": text("Connected system"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010",
                "name": text("details"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0011",
                    "name": text("client"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": client_id,
                               "type": "client_id" } }] },
            "identities": identities(client_id)
        }),
    )
    .await;
    let role_archetype = "openEHR-DEMOGRAPHIC-ROLE.connected_system.v1";
    create(
        svc,
        PartyKind::Role,
        &json!({
            "_type": "ROLE", "archetype_node_id": role_archetype,
            "archetype_details": archetyped(role_archetype), "name": text("Connected system"),
            "identities": identities("connected system"),
            "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
                           "id": { "_type": "HIER_OBJECT_ID", "value": agent.to_string() } },
            "capabilities": capabilities
        }),
    )
    .await;
    agent
}
