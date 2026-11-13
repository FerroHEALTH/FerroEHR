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

use openehr_base::containers::NonEmptyVec;
use openehr_base::prelude::{HierObjectId, ObjectId, PartyRef, TerminologyId};
use openehr_rm::prelude::{
    Agent, Capability, CodePhrase, DataValue, DvCodedText, DvIdentifier, DvText, Element, Item,
    ItemStructure, ItemTree, Party, PartyIdentity, Role,
};
use serde::Serialize;
use uuid::Uuid;

use crate::error::ServiceError;
use crate::error::refusal::Refusal;
use crate::ids::VoId;
use crate::party_binding::grants::capabilities::Actor;
use crate::party_binding::grants::operation::Operation;
use crate::party_binding::policy_mark::{CLIENT_ID, TERMINOLOGY};
use crate::service::FerroEhrService;
use crate::service::demographic::types::PartyKind;
use crate::service::ehr::service::archetyped;
use crate::service::version_update::{WriteRequest, plain_text};

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
            None => (
                self.commit_party(PartyKind::Agent, agent(grant)).await?,
                true,
            ),
        };
        let role = self
            .commit_party(PartyKind::Role, role(agent, grant))
            .await?;
        self.grants()
            .refresh()
            .await
            .map_err(|error| ServiceError::from(error.into_inner()))?;
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

    /// Commits `party` as a new party of `kind`; its container.
    async fn commit_party(&self, kind: PartyKind, party: Party) -> Result<VoId, ServiceError> {
        let created = Box::pin(self.party_create(kind, WriteRequest::new(party))).await?;
        let Some(meta) = created.meta else {
            return Err(ServiceError::Internal(Refusal::new(
                (),
                "a created party answered no version",
            )));
        };
        VoId::from_uid_head(&meta.uid)
            .map_err(|error| ServiceError::internal("reading the created party's container", error))
    }
}

fn coded(code: &str) -> DvCodedText {
    DvCodedText {
        value: code.to_owned(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
        defining_code: CodePhrase {
            terminology_id: TerminologyId {
                value: TERMINOLOGY.to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        },
    }
}

/// An `ELEMENT` at `node`, named `name`, holding `value`.
fn element(node: &str, name: DvText, value: DataValue) -> Item {
    Item::Element(Element {
        name,
        archetype_node_id: node.to_owned(),
        uid: None,
        links: None,
        archetype_details: None,
        feeder_audit: None,
        null_flavour: None,
        value: Some(value),
        null_reason: None,
    })
}

/// An `ITEM_TREE` at `node`, named `name`, holding `items`.
fn tree(node: &str, name: &str, items: Vec<Item>) -> ItemStructure {
    ItemStructure::ItemTree(Box::new(ItemTree {
        name: plain_text(name),
        archetype_node_id: node.to_owned(),
        uid: None,
        links: None,
        archetype_details: None,
        feeder_audit: None,
        items: Some(items),
    }))
}

/// The one name identity of a party named `name`.
fn identities(name: &str) -> NonEmptyVec<PartyIdentity> {
    NonEmptyVec::of(PartyIdentity {
        name: plain_text("name"),
        archetype_node_id: "at0001".to_owned(),
        uid: None,
        links: None,
        archetype_details: None,
        feeder_audit: None,
        details: tree(
            "at0002",
            "details",
            vec![element(
                "at0003",
                plain_text("name"),
                DataValue::DvText(plain_text(name)),
            )],
        ),
    })
}

/// The `AGENT` carrying `grant`'s client identifier.
fn agent(grant: &ClientGrant) -> Party {
    let client = DvIdentifier {
        issuer: Some(grant.issuer.clone()),
        assigner: None,
        id: grant.client_id.clone(),
        r#type: Some(CLIENT_ID.to_owned()),
    };
    Party::Agent(Agent {
        name: plain_text(&grant.client_id),
        archetype_node_id: AGENT_ARCHETYPE.to_owned(),
        uid: None,
        links: None,
        archetype_details: Some(archetyped(AGENT_ARCHETYPE)),
        feeder_audit: None,
        identities: identities(&grant.client_id),
        contacts: None,
        details: Some(tree(
            "at0010",
            "details",
            vec![element(
                "at0011",
                plain_text("client"),
                DataValue::DvIdentifier(client),
            )],
        )),
        relationships: None,
        languages: None,
        roles: None,
    })
}

/// The `ROLE` `agent` performs whose capability grants `grant`'s operation
/// with its scope elements.
fn role(agent: VoId, grant: &ClientGrant) -> Party {
    let code = grant.operation.code();
    let mut items = vec![element(
        "at0021",
        plain_text("operation"),
        DataValue::DvText(DvText::DvCodedText(coded(code))),
    )];
    items.extend(grant.elements.iter().map(|(name, value)| {
        element(
            "at0022",
            DvText::DvCodedText(coded(name)),
            DataValue::DvText(plain_text(value)),
        )
    }));
    Party::Role(Role {
        name: plain_text(code),
        archetype_node_id: ROLE_ARCHETYPE.to_owned(),
        uid: None,
        links: None,
        archetype_details: Some(archetyped(ROLE_ARCHETYPE)),
        feeder_audit: None,
        identities: identities(code),
        contacts: None,
        details: None,
        relationships: None,
        time_validity: None,
        performer: PartyRef {
            namespace: "demographic".to_owned(),
            r#type: "AGENT".to_owned(),
            id: ObjectId::HierObjectId(HierObjectId::from(agent.0)),
        },
        capabilities: Some(NonEmptyVec::of(Capability {
            name: plain_text("grant"),
            archetype_node_id: "at0020".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            credentials: tree("at0023", "terms", items),
            time_validity: None,
        })),
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
        let body = openehr_its::json::to_canonical_value(&role(VoId(Uuid::nil()), &grant));
        let credentials = Credentials::of(body.pointer("/capabilities/0/credentials"));
        assert_eq!(
            credentials.identified(BINDING_ISSUER),
            ["urn:example:login"]
        );
        assert_eq!(credentials.identified(TOKEN_SCOPE), ["ferroehr/scim"]);
        assert_eq!(
            body.pointer("/capabilities/0/credentials/items/0/value/defining_code/code_string"),
            Some(&serde_json::json!("scim_provision"))
        );
        assert_eq!(
            openehr_its::json::to_canonical_value(&agent(&grant))
                .pointer("/details/items/0/value/type"),
            Some(&serde_json::json!(CLIENT_ID))
        );
    }
}
