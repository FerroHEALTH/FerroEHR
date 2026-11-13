// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy parties that authorize a system client at the wire (#5432,
//! #5436), committed as the server: an `AGENT` carrying the client's
//! `client_id` under each token issuer, acting for a provider `ORGANISATION`,
//! and performing a `ROLE` whose capabilities grant operations of the
//! `ferroehr-operations` terminology. Every party is of a deployment's own
//! archetype with no template. No openEHR spec gives a capability an
//! authorization meaning: our own design.

use ferroehr::ids::VoId;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::precondition::Precondition;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use serde_json::{Value, json};

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

/// The grant to commit relayed COMPOSITIONs of `template` at the facility
/// `id` of `issuer`.
pub(crate) fn relays(template: &str, (issuer, id): (&str, &str)) -> Value {
    capability(
        "commit_composition",
        &[
            named("template", &text(template)),
            named("origin", &coded("relay")),
            json!({ "_type": "CLUSTER", "archetype_node_id": "at0031", "name": coded("facility"),
                    "items": [named("issuer", &text(issuer)), named("identifier", &text(id))] }),
        ],
    )
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

/// The write of `body`, after the version `preceding` when given.
fn write<T: serde::de::DeserializeOwned>(body: &Value, preceding: Option<&str>) -> WriteRequest<T> {
    let mut write =
        WriteRequest::new(openehr_its::json::from_canonical_value(body).expect("the body decodes"));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

/// Creates `body` as a party of `kind`, as the server; its container and
/// version uid.
async fn create(svc: &FerroEhrService, kind: PartyKind, body: &Value) -> (VoId, String) {
    let created = Box::pin(svc.party_create(kind, write(body, None)))
        .await
        .unwrap_or_else(|e| panic!("the {kind:?} commits: {e:?}"));
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    (crate::party_binding_http::vo(&uid), uid)
}

/// Commits, as the server, an `AGENT` carrying each client identifier of
/// `clients` (`(issuer, client_id)`), acting for a provider `ORGANISATION`
/// identified by `provider` (`(issuer, id)`) when given, and a `ROLE` it
/// performs holding `capabilities`; the `AGENT`'s container.
pub(crate) async fn system_client(
    svc: &FerroEhrService,
    clients: &[(&str, &str)],
    provider: Option<(&str, &str)>,
    capabilities: &[Value],
) -> VoId {
    let identifiers: Vec<Value> = clients
        .iter()
        .map(|(issuer, client_id)| {
            json!({ "_type": "ELEMENT", "archetype_node_id": "at0012", "name": text("client"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": client_id,
                               "type": "client_id" } })
        })
        .collect();
    let archetype = "openEHR-DEMOGRAPHIC-AGENT.integration_engine.v2";
    let mut agent = json!({
        "_type": "AGENT", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Integration engine"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010", "name": text("details"),
                     "items": identifiers },
        "identities": identities("integration engine")
    });
    let (vo, uid) = create(svc, PartyKind::Agent, &agent).await;
    if let Some((issuer, id)) = provider {
        let archetype = "openEHR-DEMOGRAPHIC-ORGANISATION.hospital.v1";
        let organisation = json!({
            "_type": "ORGANISATION", "archetype_node_id": archetype,
            "archetype_details": archetyped(archetype), "name": text("Hospital"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("details"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0002", "name": text("provider"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id,
                               "type": "healthcare-provider" } }] },
            "identities": identities("Hospital")
        });
        let (hospital, _) = create(svc, PartyKind::Organisation, &organisation).await;
        agent["relationships"] = json!([{
            "_type": "PARTY_RELATIONSHIP", "archetype_node_id": "at0040", "name": coded("acts_for"),
            "source": reference("AGENT", vo), "target": reference("ORGANISATION", hospital)
        }]);
        Box::pin(svc.party_update(
            PartyKind::Agent,
            vo,
            Precondition::parse(Some(uid.as_str()), None).expect("a well-formed If-Match"),
            write(&agent, Some(&uid)),
        ))
        .await
        .expect("the AGENT acts for the provider");
    }
    if !capabilities.is_empty() {
        performs(svc, ("AGENT", vo), capabilities).await;
    }
    vo
}

/// Commits, as the server, a `ROLE` of a deployment's own archetype the
/// party `performer` (its RM type and container) performs, holding
/// `capabilities`.
pub(crate) async fn performs(
    svc: &FerroEhrService,
    performer: (&str, VoId),
    capabilities: &[Value],
) {
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.integration.v1";
    let role = json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Integration"),
        "identities": identities("integration"),
        "performer": reference(performer.0, performer.1),
        "capabilities": capabilities
    });
    create(svc, PartyKind::Role, &role).await;
}

/// Maps every Basic user of `config` to a `PERSON` by its user name, a
/// binding key of the [`crate::party_binding_http::LOGIN`] issuer, beside
/// the persons issuer and party table that key belongs to; the service needs
/// [`crate::party_binding_http::mapping`], and [`bound_persons`] commits each
/// `PERSON`.
pub(crate) fn bind_basic_users(config: &mut ferroehr_rest::config::AppConfig) {
    let binding = crate::party_binding_http::config(None);
    config.auth.shared.enabled = true;
    config.auth.shared.issuer.extend(binding.auth.shared.issuer);
    config.auth.shared.party.extend(binding.auth.shared.party);
    let users: Vec<String> = config
        .auth
        .shared
        .basic
        .as_ref()
        .map(|basic| basic.users.iter().map(|user| user.user.clone()).collect())
        .unwrap_or_default();
    config.party_binding.basic_user = users
        .into_iter()
        .map(|user| ferroehr::config::binding::BasicUserKey {
            key_issuer: crate::party_binding_http::LOGIN.to_owned(),
            key: user.clone(),
            user,
        })
        .collect();
}

/// Commits the `PERSON` every Basic user of `config` binds to under
/// [`bind_basic_users`], the one `granted` names holding `capabilities`, when
/// any are given, through a `ROLE` it performs.
pub(crate) async fn bound_persons(
    svc: &FerroEhrService,
    config: &ferroehr_rest::config::AppConfig,
    (granted, capabilities): (&str, &[Value]),
) {
    for key in &config.party_binding.basic_user {
        let (uid, _) = crate::party_binding_http::professional(svc, &key.key, "532").await;
        if key.user == granted && !capabilities.is_empty() {
            performs(
                svc,
                ("PERSON", crate::party_binding_http::vo(&uid)),
                capabilities,
            )
            .await;
        }
    }
}
