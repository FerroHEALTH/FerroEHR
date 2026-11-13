// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr party-binding grant`: the operator's grant of an operation to a
//! client, before any administrator can log in.
//!
//! An administrator writes a policy party through the Demographic API, but an
//! administrator is a person bound to a `PERSON`, and the `PERSON`s of a new
//! deployment arrive through a provisioning client that itself needs a grant
//! (`scim_provision`). This step breaks that circle on the operator's
//! authority, offline, through the same commit path a Demographic API write
//! takes: an `AGENT` carrying the client identifier and a `ROLE` it performs
//! whose capability grants the operation with the scope elements given, each
//! validated, checked against `[authz.grants] allowed` and recorded with the
//! server's own committer. A client an `ACTOR` names already gets the `ROLE`
//! alone, and one that holds the operation already gets nothing. No openEHR
//! spec governs who grants a capability: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the canonical AGENT and ROLE \
              bodies the grant commits"
)]

use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::error::ServiceError;
use crate::error::refusal::Refusal;
use crate::ids::VoId;
use crate::party_binding::grants::capabilities::Actor;
use crate::party_binding::grants::operation::Operation;
use crate::party_binding::policy_mark::{CLIENT_ID, TERMINOLOGY};
use crate::service::FerroEhrService;
use crate::service::demographic::types::PartyKind;
use crate::service::version_update::WriteRequest;

/// The archetype of the `AGENT` a grant creates: FerroEHR's own, with no
/// template.
const AGENT_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-AGENT.ferroehr_client.v1";

/// The archetype of the `ROLE` a grant creates: FerroEHR's own, with no
/// template.
const ROLE_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-ROLE.ferroehr_grant.v1";

/// The grant of one operation to one client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientGrant {
    /// The issuer of the client's tokens.
    pub issuer: String,
    /// The client's OAuth 2.0 `client_id`.
    pub client_id: String,
    /// The operation granted.
    pub operation: Operation,
    /// The scope elements, each a code of the operation terminology and the
    /// text it names.
    pub elements: Vec<(String, String)>,
}

/// What a grant committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum Granted {
    /// The `AGENT` of the client and the `ROLE` granting the operation.
    Created {
        /// The new `AGENT`.
        agent: Uuid,
        /// The new `ROLE`.
        role: Uuid,
    },
    /// The `ROLE` granting the operation to the `ACTOR` naming the client.
    Added {
        /// The `ACTOR` naming the client.
        agent: Uuid,
        /// The new `ROLE`.
        role: Uuid,
    },
    /// Nothing: the client holds the operation already.
    Already {
        /// The `ACTOR` naming the client.
        agent: Uuid,
    },
}

impl FerroEhrService {
    /// Grants `grant`'s operation to its client, creating the client's
    /// `AGENT` when no `ACTOR` names it, on the operator's authority.
    ///
    /// # Errors
    /// The refusals of the commit: an operation `[authz.grants] allowed` does
    /// not admit (`403`), a scope element the operation is read by named
    /// other than once (`400`), a storage failure.
    pub async fn grant_client(&self, grant: &ClientGrant) -> Result<Granted, ServiceError> {
        let grants = self.fresh_grants().await?;
        let held = grants.capabilities(
            Actor::Client {
                issuer: &grant.issuer,
                client_id: &grant.client_id,
            },
            jiff::Timestamp::now(),
        );
        if let Some(client) = held.client()
            && held.holds(grant.operation)
        {
            return Ok(Granted::Already {
                agent: client.party,
            });
        }
        let (agent, created) = match held.client() {
            Some(client) => (VoId(client.party), false),
            None => (self.commit_party(PartyKind::Agent, &agent(grant)).await?, true),
        };
        let role = self
            .commit_party(PartyKind::Role, &role(agent, grant))
            .await?;
        self.grants().refresh().await.map_err(|error| ServiceError::from(error.into_inner()))?;
        Ok(if created {
            Granted::Created {
                agent: agent.0,
                role: role.0,
            }
        } else {
            Granted::Added {
                agent: agent.0,
                role: role.0,
            }
        })
    }

    /// Commits `body` as a new party of `kind`; its container.
    async fn commit_party(&self, kind: PartyKind, body: &Value) -> Result<VoId, ServiceError> {
        let party = openehr_its::json::from_canonical_value(body).map_err(|error| {
            ServiceError::bad_request(format!("the {kind:?} of the grant does not decode"), error)
        })?;
        let created = Box::pin(self.party_create(kind, WriteRequest::new(party))).await?;
        let Some(meta) = created.meta else {
            return Err(ServiceError::Internal(Refusal::new(
                (),
                "a created party answered no version",
            )));
        };
        VoId::from_uid_head(&meta.uid).map_err(|error| ServiceError::internal("reading the created party's container", error))
    }
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

/// The `AGENT` carrying `grant`'s client identifier.
fn agent(grant: &ClientGrant) -> Value {
    json!({
        "_type": "AGENT", "archetype_node_id": AGENT_ARCHETYPE,
        "archetype_details": archetyped(AGENT_ARCHETYPE), "name": text(&grant.client_id),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0011", "name": text("client"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": grant.issuer,
                           "id": grant.client_id, "type": CLIENT_ID } }] },
        "identities": identities(&grant.client_id)
    })
}

/// The `ROLE` `agent` performs whose capability grants `grant`'s operation
/// with its scope elements.
fn role(agent: VoId, grant: &ClientGrant) -> Value {
    let mut items = vec![json!({ "_type": "ELEMENT", "archetype_node_id": "at0021",
        "name": text("operation"), "value": coded(grant.operation.code()) })];
    items.extend(grant.elements.iter().map(|(code, value)| {
        json!({ "_type": "ELEMENT", "archetype_node_id": "at0022", "name": coded(code),
                "value": text(value) })
    }));
    json!({
        "_type": "ROLE", "archetype_node_id": ROLE_ARCHETYPE,
        "archetype_details": archetyped(ROLE_ARCHETYPE),
        "name": text(grant.operation.code()),
        "identities": identities(grant.operation.code()),
        "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
                       "id": { "_type": "HIER_OBJECT_ID", "value": agent.to_string() } },
        "capabilities": [{
            "_type": "CAPABILITY", "archetype_node_id": "at0020", "name": text("grant"),
            "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0023",
                             "name": text("terms"), "items": items }
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party_binding::grants::capabilities::Credentials;
    use crate::party_binding::grants::operation::{BINDING_ISSUER, TOKEN_SCOPE};

    #[test]
    fn the_role_codes_the_operation_and_names_each_element() {
        let grant = ClientGrant {
            issuer: "https://as.example".to_owned(),
            client_id: "provisioning".to_owned(),
            operation: Operation::ScimProvision,
            elements: vec![
                (BINDING_ISSUER.to_owned(), "urn:example:login".to_owned()),
                (TOKEN_SCOPE.to_owned(), "ferroehr/scim".to_owned()),
            ],
        };
        let body = role(VoId(Uuid::nil()), &grant);
        let credentials = Credentials::of(body.pointer("/capabilities/0/credentials"));
        assert_eq!(
            credentials.identified(BINDING_ISSUER),
            ["urn:example:login"]
        );
        assert_eq!(credentials.identified(TOKEN_SCOPE), ["ferroehr/scim"]);
        assert_eq!(
            body.pointer("/capabilities/0/credentials/items/0/value/defining_code/code_string"),
            Some(&json!("scim_provision"))
        );
        assert_eq!(
            agent(&grant).pointer("/details/items/0/value/type"),
            Some(&json!(CLIENT_ID))
        );
    }
}
