// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The federation agreements the HTTP tests commit to the CDR (#5440): a
//! partner `ORGANISATION` carrying the client identifiers of its issuer, and
//! a `ROLE` it performs whose capabilities grant
//! `federated_person_access` or `federated_organisation_access`, their terms
//! the coded elements of the `ferroehr-operations` terminology. No openEHR
//! spec gives a capability an authorization meaning: our own design.

use ferroehr::error::ServiceError;
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use serde_json::{Value, json};

/// The `[authz.grants] allowed` of a deployment that admits federation
/// agreements beside the default `commit_composition`.
pub(crate) fn ceiling() -> GrantCeiling {
    GrantCeiling::new([
        Operation::CommitComposition,
        Operation::FederatedPersonAccess,
        Operation::FederatedOrganisationAccess,
    ])
}

/// The accessor an agreement admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Accessor {
    /// Verified external persons.
    Person,
    /// The partner organisation.
    Organisation,
}

/// One agreement's terms: its accessor, the issuer of its partner and the
/// elements of its credentials.
#[derive(Debug, Clone)]
pub(crate) struct Terms {
    pub(crate) accessor: Accessor,
    pub(crate) issuer: String,
    items: Vec<Value>,
}

/// A `DV_CODED_TEXT` of the operation terminology.
fn coded(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// The `ELEMENT` the code `code` names, holding `value`.
fn element(code: &str, value: &Value) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0100", "name": coded(code),
            "value": value })
}

impl Terms {
    /// An agreement of `accessor` for the partner of `issuer`, admitting 100
    /// requests a second per caller and nothing else.
    pub(crate) fn new(accessor: Accessor, issuer: &str) -> Self {
        Self {
            accessor,
            issuer: issuer.to_owned(),
            items: Vec::new(),
        }
        .rate(100, 100)
    }

    /// The terms with the element `code` holding `value`, any it held before
    /// replaced.
    fn with(mut self, code: &str, value: &Value) -> Self {
        self.items
            .retain(|item| item["name"]["defining_code"]["code_string"] != code);
        self.items.push(element(code, value));
        self
    }

    /// The terms with `code` holding the text `value`, any it held before
    /// replaced.
    pub(crate) fn text(self, code: &str, value: &str) -> Self {
        self.with(code, &text(value))
    }

    /// The terms with one more element `code` holding the text `value`.
    pub(crate) fn also(mut self, code: &str, value: &str) -> Self {
        self.items.push(element(code, &text(value)));
        self
    }

    /// The terms admitting `requests_per_second` and `burst` per caller.
    pub(crate) fn rate(self, requests_per_second: i64, burst: i64) -> Self {
        self.with(
            "requests_per_second",
            &json!({ "_type": "DV_COUNT", "magnitude": requests_per_second }),
        )
        .with("burst", &json!({ "_type": "DV_COUNT", "magnitude": burst }))
    }

    /// The terms with `opt_out_override` set.
    pub(crate) fn overriding(self) -> Self {
        self.with(
            "opt_out_override",
            &json!({ "_type": "DV_BOOLEAN", "value": true }),
        )
    }

    /// The terms admitting a discard within `window`, an ISO 8601 duration.
    pub(crate) fn discard_window(self, window: &str) -> Self {
        self.with(
            "discard_window",
            &json!({ "_type": "DV_DURATION", "value": window }),
        )
    }

    /// The `CAPABILITY` granting these terms.
    fn capability(&self) -> Value {
        let operation = match self.accessor {
            Accessor::Person => "federated_person_access",
            Accessor::Organisation => "federated_organisation_access",
        };
        let mut items = vec![json!({ "_type": "ELEMENT", "archetype_node_id": "at0099",
                                     "name": text("operation"), "value": coded(operation) })];
        items.extend(self.items.iter().cloned());
        json!({ "_type": "CAPABILITY", "archetype_node_id": "at0050", "name": text("agreement"),
                "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0051",
                                 "name": text("credentials"), "items": items } })
    }
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

fn write<T: serde::de::DeserializeOwned>(body: &Value) -> WriteRequest<T> {
    let mut write =
        WriteRequest::new(openehr_its::json::from_canonical_value(body).expect("the body decodes"));
    write.lifecycle_state = Some("532".to_owned());
    write
}

/// Commits the partner of `issuer`, carrying a client identifier for each of
/// `clients`, and the `ROLE` it performs granting `terms`, as the server's
/// own commit; the error of the `ROLE`'s commit.
///
/// # Errors
/// The refusal of the `ROLE`'s commit.
pub(crate) async fn commit(
    svc: &FerroEhrService,
    issuer: &str,
    clients: &[&str],
    terms: &[Terms],
) -> Result<(), ServiceError> {
    let archetype = "openEHR-DEMOGRAPHIC-ORGANISATION.federation_partner.v1";
    let identifiers: Vec<Value> = clients
        .iter()
        .map(|client| {
            json!({ "_type": "ELEMENT", "archetype_node_id": "at0011", "name": text("client"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": issuer, "id": client,
                               "type": "client_id" } })
        })
        .collect();
    let partner = json!({
        "_type": "ORGANISATION", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Federation partner"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010",
                     "name": text("details"), "items": identifiers },
        "identities": identities("Federation partner")
    });
    let created = Box::pin(svc.party_create(PartyKind::Organisation, write(&partner)))
        .await
        .expect("the partner commits");
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    let vo = uid.split("::").next().expect("a container").to_owned();
    let role_archetype = "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1";
    let role = json!({
        "_type": "ROLE", "archetype_node_id": role_archetype,
        "archetype_details": archetyped(role_archetype), "name": text("Federation partner"),
        "identities": identities("federation partner"),
        "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
                       "id": { "_type": "HIER_OBJECT_ID", "value": vo } },
        "capabilities": terms.iter().map(Terms::capability).collect::<Vec<_>>()
    });
    Box::pin(svc.party_create(PartyKind::Role, write(&role)))
        .await
        .map(|_| ())
}

/// Commits a partner for every issuer `terms` name, each carrying the client
/// identifiers `clients` gives for its issuer.
pub(crate) async fn commit_all(
    svc: &FerroEhrService,
    terms: &[Terms],
    clients: impl Fn(&str) -> Vec<&'static str>,
) {
    let mut issuers: Vec<&str> = terms.iter().map(|t| t.issuer.as_str()).collect();
    issuers.sort_unstable();
    issuers.dedup();
    for issuer in issuers {
        let own: Vec<Terms> = terms
            .iter()
            .filter(|t| t.issuer == issuer)
            .cloned()
            .collect();
        commit(svc, issuer, &clients(issuer), &own)
            .await
            .expect("the agreement commits");
    }
}
