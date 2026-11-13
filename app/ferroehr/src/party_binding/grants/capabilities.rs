// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`Capabilities`]: what one caller may do by the policy grants, resolved
//! once per request, and the reading of a grant's credentials.
//!
//! The REST layer resolves the caller's capabilities after authentication
//! and runs the request inside them, so a check anywhere in the request reads
//! them with [`Capabilities::current`]. A grant is an operation of
//! [`crate::party_binding::policy_mark::TERMINOLOGY`] a `CAPABILITY` of a `ROLE` the caller performs holds,
//! in force on the request's date and instant, of an operation
//! `[authz.grants] allowed` admits. No openEHR spec gives a capability an
//! authorization meaning: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): a capability's `credentials` \
              ITEM_TREE as the party version holds it"
)]

use std::future::Future;
use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::party_binding::grants::derive::walk;
use crate::party_binding::grants::operation::Operation;
use crate::party_binding::policy_mark::coded;

tokio::task_local! {
    /// The capabilities of the caller of the running request.
    static CURRENT: Arc<Capabilities>;
}

/// Who a capability lookup is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor<'a> {
    /// A client, by the issuer of its token and its OAuth 2.0 `client_id`:
    /// the `ACTOR` carrying that client identifier.
    Client {
        /// The token issuer.
        issuer: &'a str,
        /// The token's client.
        client_id: &'a str,
    },
    /// A party by its container, such as the `PERSON` a login is bound to.
    Party(Uuid),
}

/// The state of a client `ACTOR`'s head (RM common
/// `master06-change_control_package.adoc` §Version Lifecycle, §Abandoned and
/// Inactive States).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientState {
    /// `532|complete|`.
    Complete,
    /// `553|incomplete|`.
    Incomplete,
    /// `800|inactive|`: the `ACTOR` holds no grant.
    Inactive,
    /// `801|abandoned|`: the `ACTOR` holds no grant.
    Abandoned,
}

impl ClientState {
    /// The state a `version_lifecycle_state` code names.
    pub(crate) fn of(code: &str) -> Self {
        match code {
            "553" => Self::Incomplete,
            "800" => Self::Inactive,
            "801" => Self::Abandoned,
            // NOTE: ck_binding_client_lifecycle_state admits only 532, 553, 800
            // and 801, so every other code is `532|complete|`.
            _ => Self::Complete,
        }
    }

    /// Whether an `ACTOR` in this state holds its grants.
    #[must_use]
    pub const fn grants(self) -> bool {
        matches!(self, Self::Complete | Self::Incomplete)
    }
}

/// The `ACTOR` a client identifier names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Client {
    /// The `ACTOR`'s container.
    pub party: Uuid,
    /// The state of its head.
    pub state: ClientState,
}

/// One grant in force: an operation a `CAPABILITY` of a `ROLE` grants.
#[derive(Clone, PartialEq)]
pub struct Grant {
    operation: Operation,
    role: Uuid,
    credentials: Option<Arc<Value>>,
}

impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant")
            .field("operation", &self.operation)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl Grant {
    /// The grant of `operation` by the `ROLE` `role`, holding `credentials`.
    pub(crate) fn new(operation: Operation, role: Uuid, credentials: Option<Arc<Value>>) -> Self {
        Self {
            operation,
            role,
            credentials,
        }
    }

    /// The operation granted.
    #[must_use]
    pub fn operation(&self) -> Operation {
        self.operation
    }

    /// The container of the `ROLE` that carries it.
    #[must_use]
    pub fn role(&self) -> Uuid {
        self.role
    }

    /// Its credentials, read by the elements the operation terminology names.
    #[must_use]
    pub fn credentials(&self) -> Credentials<'_> {
        Credentials(self.credentials.as_deref())
    }
}

/// The `credentials` of a grant, or one `CLUSTER` of them, read by the
/// elements [`crate::party_binding::policy_mark::TERMINOLOGY`] names.
///
/// An element or a cluster is read by the code of its `name`, a
/// `DV_CODED_TEXT` of the operation terminology, wherever it sits in the
/// tree, so a check reads a grant's scope (such as `template` or `facility`)
/// in any archetype that codes it.
#[derive(Debug, Clone, Copy)]
pub struct Credentials<'a>(Option<&'a Value>);

impl<'a> Credentials<'a> {
    /// The credentials `tree`, an `ITEM_STRUCTURE` as the capability holds it.
    pub(crate) fn of(tree: Option<&'a Value>) -> Self {
        Self(tree)
    }

    /// The items below this tree whose `name` is `code`, in document order.
    #[must_use]
    pub fn named(self, code: &str) -> Vec<&'a Value> {
        let mut out = Vec::new();
        if let Some(tree) = self.0.and_then(|tree| tree.get("items")) {
            walk(tree, &mut |item| {
                if item.get("name").and_then(coded) == Some(code) {
                    out.push(item);
                }
            });
        }
        out
    }

    /// The `value` of each element `code` names.
    fn values(self, code: &str) -> impl Iterator<Item = &'a Value> + use<'a> {
        self.named(code)
            .into_iter()
            .filter_map(|item| item.get("value"))
    }

    /// The text of each `DV_TEXT` or `DV_CODED_TEXT` element `code` names,
    /// such as the templates a grant writes (`template`).
    #[must_use]
    pub fn texts(self, code: &str) -> Vec<&'a str> {
        self.values(code)
            .filter_map(|value| value.get("value").and_then(Value::as_str))
            .collect()
    }

    /// What each element `code` names identifies: the text of a `DV_TEXT` or
    /// `DV_CODED_TEXT` value, or the `id` of a `DV_IDENTIFIER` value, trimmed,
    /// such as the origin of a [`Operation::DataOrigin`] grant.
    #[must_use]
    pub fn identified(self, code: &str) -> Vec<&'a str> {
        self.values(code)
            .filter_map(|value| {
                value
                    .get("value")
                    .or_else(|| value.get("id"))
                    .and_then(Value::as_str)
            })
            .map(str::trim)
            .collect()
    }

    /// The code of each `DV_CODED_TEXT` element `code` names.
    #[must_use]
    pub fn codes(self, code: &str) -> Vec<&'a str> {
        self.values(code)
            .filter_map(|value| value.get("defining_code")?.get("code_string")?.as_str())
            .collect()
    }

    /// The magnitude of the first `DV_COUNT` element `code` names.
    #[must_use]
    pub fn count(self, code: &str) -> Option<i64> {
        self.values(code)
            .find_map(|value| value.get("magnitude").and_then(Value::as_i64))
    }

    /// The value of the first `DV_BOOLEAN` element `code` names.
    #[must_use]
    pub fn boolean(self, code: &str) -> Option<bool> {
        self.values(code)
            .find_map(|value| value.get("value").and_then(Value::as_bool))
    }

    /// Each `CLUSTER` `code` names, read the same way, such as a facility.
    #[must_use]
    pub fn clusters(self, code: &str) -> Vec<Credentials<'a>> {
        self.named(code)
            .into_iter()
            .filter(|item| item.get("items").is_some())
            .map(|item| Credentials(Some(item)))
            .collect()
    }
}

/// What one caller may do: the `ACTOR` its client identifier names, and the
/// grants in force on the request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Capabilities {
    client: Option<Client>,
    grants: Vec<Grant>,
}

impl Capabilities {
    /// The capabilities of the client `client` or of a party, holding
    /// `grants`.
    pub(crate) fn new(client: Option<Client>, grants: Vec<Grant>) -> Self {
        Self { client, grants }
    }

    /// The capabilities of the caller of the running request; none outside
    /// a request, for a caller who is no client and binds to no party, and
    /// for one nothing grants an operation.
    #[must_use]
    pub fn current() -> Arc<Self> {
        CURRENT.try_with(Arc::clone).unwrap_or_default()
    }

    /// The `ACTOR` the caller's client identifier names; `None` for a party,
    /// or a client no `ACTOR` carries.
    #[must_use]
    pub fn client(&self) -> Option<Client> {
        self.client
    }

    /// Every grant in force.
    #[must_use]
    pub fn grants(&self) -> &[Grant] {
        &self.grants
    }

    /// The grants in force of `operation`.
    pub fn of(&self, operation: Operation) -> impl Iterator<Item = &Grant> {
        self.grants
            .iter()
            .filter(move |grant| grant.operation == operation)
    }

    /// Whether a grant of `operation` is in force.
    #[must_use]
    pub fn holds(&self, operation: Operation) -> bool {
        self.of(operation).next().is_some()
    }
}

/// Runs `fut` as a request whose caller has `capabilities`, which
/// [`Capabilities::current`] reads.
pub async fn with_capabilities<F: Future>(capabilities: Arc<Capabilities>, fut: F) -> F::Output {
    CURRENT.scope(capabilities, fut).await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::party_binding::policy_mark::TERMINOLOGY;

    fn named(code: &str, value: &Value) -> Value {
        json!({"_type": "ELEMENT", "archetype_node_id": "at1",
               "name": {"_type": "DV_CODED_TEXT", "value": code,
                        "defining_code": {"terminology_id": {"value": TERMINOLOGY},
                                          "code_string": code}},
               "value": value})
    }

    #[test]
    fn credentials_read_by_coded_name_anywhere_and_through_a_cluster() {
        let mut facility = named("facility", &Value::Null);
        facility["items"] = json!([named(
            "subject_namespace",
            &json!({"_type": "DV_TEXT", "value": "urn:x"})
        )]);
        let tree = json!({"_type": "ITEM_TREE", "items": [
            named("template", &json!({"_type": "DV_TEXT", "value": "T.v1"})),
            {"_type": "CLUSTER", "items": [named("template", &json!({"_type": "DV_TEXT", "value": "U.v1"}))]},
            facility,
            {"_type": "ELEMENT", "name": {"_type": "DV_TEXT", "value": "template"},
             "value": {"_type": "DV_TEXT", "value": "not coded"}}
        ]});
        let grant = Grant::new(Operation::ImportEhr, Uuid::nil(), Some(Arc::new(tree)));
        let credentials = grant.credentials();
        assert_eq!(credentials.texts("template"), vec!["T.v1", "U.v1"]);
        assert_eq!(credentials.identified("template"), vec!["T.v1", "U.v1"]);
        let [facility] = credentials.clusters("facility")[..] else {
            panic!("one facility");
        };
        assert_eq!(facility.texts("subject_namespace"), vec!["urn:x"]);
        assert_eq!(
            Grant::new(Operation::ImportEhr, Uuid::nil(), None)
                .credentials()
                .texts("template"),
            Vec::<&str>::new()
        );
    }

    #[tokio::test]
    async fn the_current_capabilities_are_the_requests() {
        assert_eq!(*Capabilities::current(), Capabilities::default());
        let granted = Arc::new(Capabilities::new(
            None,
            vec![Grant::new(Operation::ReadSubjectAudit, Uuid::nil(), None)],
        ));
        let held = with_capabilities(Arc::clone(&granted), async {
            Capabilities::current().holds(Operation::ReadSubjectAudit)
        })
        .await;
        assert!(held);
    }
}
