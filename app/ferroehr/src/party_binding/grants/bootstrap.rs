// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr party-binding grant`: the operator's grant of an operation to a
//! client or to the `PERSON` of a Basic user, before any administrator can
//! log in.
//!
//! An administrator writes a policy party through the Demographic API, but an
//! administrator is a person bound to a `PERSON`, and the `PERSON`s of a new
//! deployment arrive through a provisioning client that itself needs a grant
//! (`scim_provision`). This step breaks that circle on the operator's
//! authority, offline, through the same commit path a Demographic API write
//! takes: an `AGENT` of the shipped system definition carrying the client
//! identifier and a `ROLE` of the shipped grant definition it performs,
//! whose capability grants the operation with the scope elements given, each
//! validated, checked against `[authz.grants] allowed` and recorded with the
//! server's own committer. A client an `ACTOR` names already gets the `ROLE`
//! alone, and one that holds the operation already gets nothing. A Basic
//! user's `PERSON`, which its provisioning created, gets a `ROLE` it performs
//! the same way, such as the `hold_scopes` a Basic user's scopes come from.
//! No openEHR spec governs who grants a capability: our own design.

use openehr_base::containers::NonEmptyVec;
use openehr_base::prelude::{
    ArchetypeId, HierObjectId, ObjectId, PartyRef, TemplateId, TerminologyId,
};
use openehr_rm::prelude::{
    Agent, Archetyped, Capability, CodePhrase, DataValue, DvCodedText, DvIdentifier, DvText,
    Element, Item, ItemStructure, ItemTree, Party, PartyIdentity, Role,
};
use serde::Serialize;
use uuid::Uuid;

use crate::error::ServiceError;
use crate::error::refusal::Refusal;
use crate::ids::VoId;
use crate::party_binding::grants::capabilities::Actor;
use crate::party_binding::grants::operation::Operation;
use crate::party_binding::policy::{AGENT_ARCHETYPE, PolicyRole};
use crate::party_binding::policy_mark::{CLIENT_ID, TERMINOLOGY};
use crate::service::FerroEhrService;
use crate::service::demographic::types::PartyKind;
use crate::service::version_update::{WriteRequest, plain_text};

/// The template of the `AGENT` a grant creates: the shipped system `AGENT`
/// ([`crate::party_binding::policy::AGENT_ARCHETYPE`]).
const AGENT_TEMPLATE: &str = "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0";

/// The template of the `ROLE` a grant creates: the shipped grant `ROLE`
/// ([`PolicyRole::Grant`]).
const ROLE_TEMPLATE: &str = "openEHR-DEMOGRAPHIC-ROLE.grant.v1.0.0";

/// The grant of one operation to one client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientGrant {
    /// The issuer of the client's tokens.
    pub issuer: String,
    /// The client's OAuth 2.0 `client_id`.
    pub client_id: String,
    /// The operation granted.
    pub operation: Operation,
    /// The scope elements.
    pub elements: Vec<GrantElement>,
}

/// One scope element of a grant, named by a code of the draft access
/// terminology, with its value typed as the element is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantElement {
    /// An element whose value is a `DV_TEXT`, such as a `binding_issuer`.
    Text {
        /// The element's code.
        code: String,
        /// The text.
        text: String,
    },
    /// An element whose value is a `DV_IDENTIFIER`, such as the `provider`
    /// of a `write_care_relationship` grant.
    Identifier {
        /// The element's code.
        code: String,
        /// The identifier's issuer.
        issuer: String,
        /// The identifier.
        id: String,
    },
}

/// The grant of one operation to the `PERSON` a Basic user binds to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonGrant {
    /// The `PERSON`'s container.
    pub person: Uuid,
    /// The operation granted.
    pub operation: Operation,
    /// The scope elements.
    pub elements: Vec<GrantElement>,
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
    /// The `ROLE` granting the operation to the `ACTOR` naming the client,
    /// or to the Basic user's `PERSON`.
    Added {
        /// The `ACTOR` naming the client, or the `PERSON`.
        agent: Uuid,
        /// The new `ROLE`.
        role: Uuid,
    },
    /// Nothing: the client or the `PERSON` holds the operation already.
    Already {
        /// The `ACTOR` naming the client, or the `PERSON`.
        agent: Uuid,
    },
}

impl FerroEhrService {
    /// Grants `grant`'s operation to its client, creating the client's
    /// `AGENT` when no `ACTOR` names it, on the operator's authority.
    ///
    /// The `AGENT` and the `ROLE` are written under the shipped definitions,
    /// which the grant stores first when no definition of their ids is
    /// stored, as a boot does.
    ///
    /// # Errors
    /// A shipped definition the definition store refuses, and the refusals
    /// of the commit: an operation `[authz.grants] allowed` does
    /// not admit (`403`), a scope element the operation is read by named
    /// other than once (`400`), a storage failure.
    pub async fn grant_client(&self, grant: &ClientGrant) -> Result<Granted, ServiceError> {
        self.seed_default_definitions().await?;
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
            .commit_party(
                PartyKind::Role,
                role(("AGENT", agent), grant.operation, &grant.elements),
            )
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

    /// Grants `grant`'s operation to its `PERSON` through a `ROLE` it
    /// performs, on the operator's authority; nothing when the `PERSON`
    /// holds the operation already.
    ///
    /// # Errors
    /// The refusals of the commit, as [`Self::grant_client`]'s, and the
    /// refusal of a `ROLE` whose performer is no `PERSON` of this store.
    pub async fn grant_person(&self, grant: &PersonGrant) -> Result<Granted, ServiceError> {
        self.seed_default_definitions().await?;
        let grants = self.fresh_grants().await?;
        let held = grants.capabilities(Actor::Party(grant.person), jiff::Timestamp::now());
        if held.holds(grant.operation) {
            return Ok(Granted::Already {
                agent: grant.person,
            });
        }
        let role = self
            .commit_party(
                PartyKind::Role,
                role(
                    ("PERSON", VoId(grant.person)),
                    grant.operation,
                    &grant.elements,
                ),
            )
            .await?;
        self.grants()
            .refresh()
            .await
            .map_err(|error| ServiceError::from(error.into_inner()))?;
        Ok(Granted::Added {
            agent: grant.person,
            role: role.0,
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
    Item::Element(Box::new(Element {
        name,
        archetype_node_id: node.to_owned(),
        uid: None,
        links: None,
        archetype_details: None,
        feeder_audit: None,
        null_flavour: None,
        value: Some(Box::new(value)),
        null_reason: None,
    }))
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

/// The archetype details of a party of the archetype `archetype_id` written
/// under the template `template_id`.
fn archetyped(archetype_id: &str, template_id: &str) -> Archetyped {
    Archetyped {
        archetype_id: ArchetypeId {
            value: archetype_id.to_owned(),
        },
        template_id: Some(TemplateId {
            value: template_id.to_owned(),
        }),
        rm_version: openehr_rm::Generation::default().spec_version().to_owned(),
    }
}

/// The one name identity of a party named `name`, at the node ids `nodes`
/// of the identity, its tree and its element.
fn identities(name: &str, nodes: [&str; 3]) -> NonEmptyVec<PartyIdentity> {
    let [identity, details, element_node] = nodes;
    NonEmptyVec::of(PartyIdentity {
        name: plain_text("name"),
        archetype_node_id: identity.to_owned(),
        uid: None,
        links: None,
        archetype_details: None,
        feeder_audit: None,
        details: tree(
            details,
            "details",
            vec![element(
                element_node,
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
    Party::Agent(Box::new(Agent {
        name: plain_text(&grant.client_id),
        archetype_node_id: AGENT_ARCHETYPE.to_owned(),
        uid: None,
        links: None,
        archetype_details: Some(Box::new(archetyped(AGENT_ARCHETYPE, AGENT_TEMPLATE))),
        feeder_audit: None,
        identities: identities(&grant.client_id, ["id4", "id5", "id6"]),
        contacts: None,
        details: Some(tree(
            "id2",
            "details",
            vec![element(
                "id3",
                plain_text("client"),
                DataValue::DvIdentifier(client),
            )],
        )),
        relationships: None,
        languages: None,
        roles: None,
    }))
}

/// The `ROLE` the party `performer` (its RM type and container) performs,
/// whose capability grants `operation` with the scope `elements`.
fn role(performer: (&str, VoId), operation: Operation, elements: &[GrantElement]) -> Party {
    let code = operation.code();
    let mut items = vec![element(
        "id90",
        plain_text("operation"),
        DataValue::DvText(DvText::DvCodedText(coded(code))),
    )];
    items.extend(elements.iter().map(|named| match named {
        GrantElement::Text { code, text } => element(
            "id9",
            DvText::DvCodedText(coded(code)),
            DataValue::DvText(plain_text(text)),
        ),
        GrantElement::Identifier { code, issuer, id } => element(
            "id10",
            DvText::DvCodedText(coded(code)),
            DataValue::DvIdentifier(DvIdentifier {
                issuer: Some(issuer.clone()),
                assigner: None,
                id: id.clone(),
                r#type: None,
            }),
        ),
    }));
    let archetype = PolicyRole::Grant.archetype_id();
    Party::Role(Box::new(Role {
        name: plain_text(code),
        archetype_node_id: archetype.to_owned(),
        uid: None,
        links: None,
        archetype_details: Some(Box::new(archetyped(archetype, ROLE_TEMPLATE))),
        feeder_audit: None,
        identities: identities(code, ["id2", "id3", "id4"]),
        contacts: None,
        details: None,
        relationships: None,
        time_validity: None,
        performer: PartyRef {
            namespace: "demographic".to_owned(),
            r#type: performer.0.to_owned(),
            id: ObjectId::HierObjectId(HierObjectId::from(performer.1.0)),
        },
        capabilities: Some(NonEmptyVec::of(Capability {
            name: plain_text("grant"),
            archetype_node_id: "id6".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            credentials: tree("id8", "terms", items),
            time_validity: None,
        })),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party_binding::grants::caller::care_of::ProviderKey;
    use crate::party_binding::grants::capabilities::Credentials;
    use crate::party_binding::grants::operation::{BINDING_ISSUER, PROVIDER, TOKEN_SCOPE};

    #[test]
    fn the_role_codes_the_operation_and_names_each_element() {
        let grant = ClientGrant {
            issuer: "https://as.example".to_owned(),
            client_id: "provisioning".to_owned(),
            operation: Operation::ScimProvision,
            elements: vec![
                GrantElement::Text {
                    code: BINDING_ISSUER.to_owned(),
                    text: "urn:example:login".to_owned(),
                },
                GrantElement::Text {
                    code: TOKEN_SCOPE.to_owned(),
                    text: "ferroehr/scim".to_owned(),
                },
                GrantElement::Identifier {
                    code: PROVIDER.to_owned(),
                    issuer: "urn:example:providers".to_owned(),
                    id: "hospital".to_owned(),
                },
            ],
        };
        let body = openehr_its::json::to_canonical_value(&role(
            ("AGENT", VoId(Uuid::nil())),
            grant.operation,
            &grant.elements,
        ));
        let credentials = Credentials::of(body.pointer("/capabilities/0/credentials"));
        assert_eq!(
            credentials.identified(BINDING_ISSUER),
            ["urn:example:login"]
        );
        assert_eq!(credentials.identified(TOKEN_SCOPE), ["ferroehr/scim"]);
        assert_eq!(
            credentials.identifiers(PROVIDER),
            [ProviderKey::new("urn:example:providers", "hospital")]
        );
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

    /// The `AGENT` and the `ROLE` are written under the shipped, neutral
    /// definitions, each a policy kind validated against its own template.
    #[test]
    fn a_grant_writes_under_the_shipped_definitions() {
        use crate::party_binding::policy::PolicyKind;
        let grant = ClientGrant {
            issuer: "https://as.example".to_owned(),
            client_id: "provisioning".to_owned(),
            operation: Operation::ScimProvision,
            elements: Vec::new(),
        };
        let agent = openehr_its::json::to_canonical_value(&agent(&grant));
        let role = openehr_its::json::to_canonical_value(&role(
            ("AGENT", VoId(Uuid::nil())),
            grant.operation,
            &grant.elements,
        ));
        for (party, archetype, template, kind) in [
            (&agent, AGENT_ARCHETYPE, AGENT_TEMPLATE, PolicyKind::Agent),
            (
                &role,
                PolicyRole::Grant.archetype_id(),
                ROLE_TEMPLATE,
                PolicyKind::Role(PolicyRole::Grant),
            ),
        ] {
            assert_eq!(party["archetype_node_id"], archetype);
            assert_eq!(
                party["archetype_details"]["archetype_id"]["value"],
                archetype
            );
            assert_eq!(party["archetype_details"]["template_id"]["value"], template);
            assert_eq!(PolicyKind::of(archetype), Some(kind));
            assert!(
                crate::party_binding::policy::DEFINITIONS
                    .iter()
                    .any(|(file, _)| file.strip_suffix(".adls") == Some(template)),
                "{template} is shipped"
            );
        }
        assert_eq!(
            agent.pointer("/details/archetype_node_id"),
            Some(&serde_json::json!("id2"))
        );
        assert_eq!(
            role.pointer("/capabilities/0/credentials/items/0/archetype_node_id"),
            Some(&serde_json::json!("id90"))
        );
    }
}
