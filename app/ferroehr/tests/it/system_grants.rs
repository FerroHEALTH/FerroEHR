// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy parties a system client is authorized by, committed as the
//! server, and the request scope that runs a commit as that client (#5432,
//! #5436).
//!
//! A system client is an `AGENT` carrying its `client_id`, acting for a
//! provider `ORGANISATION` by an acts-for `PARTY_RELATIONSHIP`, and
//! performing a `ROLE` whose capabilities grant operations of the
//! `ferroehr-operations` terminology. Every party is of a deployment's own
//! archetype with no template. No openEHR spec gives a capability an
//! authorization meaning: our own design.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::fixtures::{if_match, object, written_vo};
use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::capabilities::{Actor, Capabilities, with_capabilities};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

/// The issuer of the system clients' tokens.
pub(crate) const MACHINES: &str = "https://machines.example";
/// The facility identifier issuer the clients commit for.
pub(crate) const FACILITIES: &str = "urn:oid:2.999.1";
/// The issuer of the device identifiers.
pub(crate) const DEVICES: &str = "urn:oid:2.999.9";
/// The issuer of the provider identifier.
pub(crate) const PROVIDERS: &str = "urn:oid:2.999.7";

/// A `DV_TEXT` of `value`.
pub(crate) fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// A `DV_CODED_TEXT` of the operation terminology.
pub(crate) fn coded(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

/// A credentials element named by the terminology's `code`, holding `value`.
pub(crate) fn named(code: &str, value: &Value) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0030", "name": coded(code),
            "value": value })
}

/// A credentials `CLUSTER` named by the terminology's `code`.
pub(crate) fn cluster(code: &str, items: &[Value]) -> Value {
    json!({ "_type": "CLUSTER", "archetype_node_id": "at0031", "name": coded(code),
            "items": items })
}

/// The `ARCHETYPED` of the deployment's own `archetype`, with no template.
fn archetyped(archetype: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" })
}

/// A name identity of `name`.
fn identities(name: &str) -> Value {
    json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": text("name"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                        "value": text(name) }] } }])
}

/// A `PARTY_REF` of `kind` to the container `vo`.
fn reference(kind: &str, vo: VoId) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": kind,
            "id": { "_type": "HIER_OBJECT_ID", "value": vo.to_string() } })
}

/// A capability granting `operation`, with the further credentials `items`.
pub(crate) fn capability(operation: &str, items: &[Value]) -> Value {
    let mut all = vec![json!({ "_type": "ELEMENT", "archetype_node_id": "at0021",
        "name": text("operation"), "value": coded(operation) })];
    all.extend_from_slice(items);
    json!({
        "_type": "CAPABILITY", "archetype_node_id": "at0020", "name": text("grant"),
        "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0022",
                         "name": text("terms"), "items": all }
    })
}

/// The grant to commit COMPOSITIONs of `template` at the facility `ward-7`
/// of [`FACILITIES`], from `origins`, naming devices of [`DEVICES`].
pub(crate) fn writes(template: &str, origins: &[&str]) -> Value {
    let mut items = vec![
        named("template", &text(template)),
        cluster(
            "facility",
            &[
                named("issuer", &text(FACILITIES)),
                named("identifier", &text("ward-7")),
            ],
        ),
        named("device_identifier_issuer", &text(DEVICES)),
    ];
    items.extend(origins.iter().map(|origin| named("origin", &coded(origin))));
    capability("commit_composition", &items)
}

/// Creates `body` as a party of `kind`, as the server; its container and
/// version uid.
async fn create(svc: &FerroEhrService, kind: PartyKind, body: &Value) -> (VoId, String) {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    let created = Box::pin(svc.party_create(kind, write))
        .await
        .unwrap_or_else(|e| panic!("the {kind:?} commits: {e:?}"));
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    (written_vo(&created), uid)
}

/// Commits, as the server, the `AGENT` of the client `client_id` of
/// [`MACHINES`] acting for a provider `ORGANISATION` when `provider`, and a
/// `ROLE` it performs holding `capabilities`; the `AGENT`'s container.
pub(crate) async fn system_client(
    svc: &FerroEhrService,
    client_id: &str,
    provider: bool,
    capabilities: &[Value],
) -> VoId {
    let agent = |relationships: Option<Value>| {
        let archetype = "openEHR-DEMOGRAPHIC-AGENT.integration_engine.v2";
        let mut agent = json!({
            "_type": "AGENT", "archetype_node_id": archetype,
            "archetype_details": archetyped(archetype), "name": text("Integration engine"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010", "name": text("details"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0012", "name": text("client"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": MACHINES, "id": client_id,
                               "type": "client_id" } }] },
            "identities": identities(client_id)
        });
        if let Some(relationships) = relationships {
            agent["relationships"] = relationships;
        }
        agent
    };
    let (vo, uid) = create(svc, PartyKind::Agent, &agent(None)).await;
    if provider {
        let archetype = "openEHR-DEMOGRAPHIC-ORGANISATION.hospital.v1";
        let organisation = json!({
            "_type": "ORGANISATION", "archetype_node_id": archetype,
            "archetype_details": archetyped(archetype), "name": text("Hospital"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("details"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0002", "name": text("provider"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": PROVIDERS, "id": "hospital-1",
                               "type": "healthcare-provider" } }] },
            "identities": identities("Hospital")
        });
        let (hospital, _) = create(svc, PartyKind::Organisation, &organisation).await;
        let acting = agent(Some(json!([{
            "_type": "PARTY_RELATIONSHIP", "archetype_node_id": "at0040", "name": coded("acts_for"),
            "source": reference("AGENT", vo), "target": reference("ORGANISATION", hospital)
        }])));
        let mut write = WriteRequest::new(crate::typed_body::typed(&acting));
        write.lifecycle_state = Some("532".to_owned());
        write.preceding = Some(uid.parse().expect("an OBJECT_VERSION_ID"));
        Box::pin(svc.party_update(
            PartyKind::Agent,
            object(vo.to_string()),
            if_match(&uid),
            write,
        ))
        .await
        .expect("the AGENT acts for the provider");
    }
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.integration.v1";
    let role = json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Integration"),
        "identities": identities("integration"),
        "performer": reference("AGENT", vo),
        "capabilities": capabilities
    });
    if !capabilities.is_empty() {
        create(svc, PartyKind::Role, &role).await;
    }
    vo
}

/// A system caller: the identity of its token and the capabilities its
/// client resolves to.
#[derive(Debug, Clone)]
pub(crate) struct System {
    /// The committer identity of its token.
    pub(crate) identity: CommitterIdentity,
    /// What the policy parties grant it.
    pub(crate) granted: Arc<Capabilities>,
}

/// The system caller of the client `client_id` of [`MACHINES`] on `svc`,
/// after `svc` read the grant rows.
pub(crate) async fn system(svc: &FerroEhrService, client_id: &str) -> System {
    svc.grants().refresh().await.expect("the grants read");
    let granted = Arc::new(svc.grants().capabilities(
        Actor::Client {
            issuer: MACHINES,
            client_id,
        },
        jiff::Timestamp::now(),
    ));
    let identity = CommitterIdentity {
        subject: client_id.to_owned(),
        id_type: "oauth2",
        issuer: Some(MACHINES.to_owned()),
        client: Some(client_id.to_owned()),
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: true,
        agent: granted.client().map(|client| client.party),
        external: None,
    };
    System { identity, granted }
}

/// Runs `fut` as the request of `caller`; a request with no committer when
/// `None`. The request is boxed, as a commit's future is large.
pub(crate) fn run<'a, F: Future + 'a>(
    caller: Option<&System>,
    fut: F,
) -> std::pin::Pin<Box<dyn Future<Output = F::Output> + 'a>> {
    match caller {
        Some(caller) => Box::pin(with_capabilities(
            Arc::clone(&caller.granted),
            with_committer(Some(caller.identity.clone()), fut),
        )),
        None => Box::pin(with_committer(None, fut)),
    }
}
