// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`Capabilities`]: what one caller may do by the policy parties, resolved
//! once per request, and the typed reading of a grant's credentials.
//!
//! The REST layer resolves the caller's capabilities after authentication
//! and runs the request inside them, so a check anywhere in the request reads
//! them with [`Capabilities::current`]. A grant is a `CAPABILITY` of a policy
//! `ROLE` the caller performs that holds on the request's date and instant,
//! of a kind `[authz.grants] allowed` admits. No openEHR spec gives a
//! capability an authorization meaning: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): a capability's `credentials` \
              ITEM_TREE as the party version holds it"
)]

use std::future::Future;
use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::party_binding::grants::derive::is_node;
use crate::party_binding::grants::kind::GrantKind;

tokio::task_local! {
    /// The capabilities of the caller of the running request.
    static CURRENT: Arc<Capabilities>;
}

/// Who a capability lookup is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor<'a> {
    /// A system client, by the issuer of its token and its OAuth 2.0
    /// `client_id`: the `AGENT` carrying that client identifier.
    System {
        /// The token issuer.
        issuer: &'a str,
        /// The token's client.
        client_id: &'a str,
    },
    /// A party by its container, such as the `PERSON` a login is bound to.
    Party(Uuid),
}

/// The state of a system `AGENT`'s head (RM common
/// `master06-change_control_package.adoc` §Version Lifecycle, §Abandoned and
/// Inactive States).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// `532|complete|`.
    Complete,
    /// `553|incomplete|`.
    Incomplete,
    /// `800|inactive|`: the `AGENT` holds no grant.
    Inactive,
    /// `801|abandoned|`: the `AGENT` holds no grant.
    Abandoned,
}

impl AgentState {
    /// The state a `version_lifecycle_state` code names.
    pub(crate) fn of(code: &str) -> Self {
        match code {
            "553" => Self::Incomplete,
            "800" => Self::Inactive,
            "801" => Self::Abandoned,
            // NOTE: ck_binding_agent_lifecycle_state admits only 532, 553, 800
            // and 801, so every other code is `532|complete|`.
            _ => Self::Complete,
        }
    }

    /// Whether an `AGENT` in this state holds its grants.
    #[must_use]
    pub const fn grants(self) -> bool {
        matches!(self, Self::Complete | Self::Incomplete)
    }
}

/// The system `AGENT` a client identifier names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Agent {
    /// The `AGENT`'s container.
    pub party: Uuid,
    /// The state of its head.
    pub state: AgentState,
}

/// One grant in force: a `CAPABILITY` of a policy `ROLE`.
#[derive(Clone, PartialEq)]
pub struct Grant {
    kind: GrantKind,
    role: Uuid,
    credentials: Option<Arc<Value>>,
}

impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant")
            .field("kind", &self.kind)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl Grant {
    /// The grant `kind` of the `ROLE` `role`, holding `credentials`.
    pub(crate) fn new(kind: GrantKind, role: Uuid, credentials: Option<Arc<Value>>) -> Self {
        Self {
            kind,
            role,
            credentials,
        }
    }

    /// The kind of grant.
    #[must_use]
    pub fn kind(&self) -> GrantKind {
        self.kind
    }

    /// The container of the `ROLE` that carries it.
    #[must_use]
    pub fn role(&self) -> Uuid {
        self.role
    }

    /// Its credentials, read by the node ids of its `ROLE` archetype.
    #[must_use]
    pub fn credentials(&self) -> Credentials<'_> {
        Credentials(self.credentials.as_deref())
    }
}

/// The `credentials` of a grant, or one `CLUSTER` of them, read by node id.
///
/// The node ids are the contract of the policy `ROLE` archetypes (the book's
/// "Operational policy" page); a node of a specialised archetype (`id9.1`)
/// is read as the node it redefines.
#[derive(Debug, Clone, Copy)]
pub struct Credentials<'a>(Option<&'a Value>);

impl<'a> Credentials<'a> {
    /// The items whose node is `node`, in document order.
    fn items(self, node: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
        self.0
            .and_then(|tree| tree.get("items"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(move |item| {
                item.get("archetype_node_id")
                    .and_then(Value::as_str)
                    .is_some_and(|held| is_node(held, node))
            })
    }

    /// The `value` of each element `node`.
    fn values(self, node: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
        self.items(node).filter_map(|item| item.get("value"))
    }

    /// The text of each `DV_TEXT` or `DV_CODED_TEXT` element `node`, such
    /// as the templates a system client writes (`id9`).
    #[must_use]
    pub fn texts(self, node: &'a str) -> Vec<&'a str> {
        self.values(node)
            .filter_map(|value| value.get("value").and_then(Value::as_str))
            .collect()
    }

    /// The code of each `DV_CODED_TEXT` element `node`, such as the
    /// operations of a data migration (`id9`: `at20`, `at21`, `at22`).
    #[must_use]
    pub fn codes(self, node: &'a str) -> Vec<&'a str> {
        self.values(node)
            .filter_map(|value| value.get("defining_code")?.get("code_string")?.as_str())
            .collect()
    }

    /// The magnitude of the first `DV_COUNT` element `node`.
    #[must_use]
    pub fn count(self, node: &'a str) -> Option<i64> {
        self.values(node)
            .find_map(|value| value.get("magnitude").and_then(Value::as_i64))
    }

    /// The value of the first `DV_BOOLEAN` element `node`.
    #[must_use]
    pub fn boolean(self, node: &'a str) -> Option<bool> {
        self.values(node)
            .find_map(|value| value.get("value").and_then(Value::as_bool))
    }

    /// The ISO 8601 text of the first `DV_DURATION` element `node`.
    #[must_use]
    pub fn duration(self, node: &'a str) -> Option<&'a str> {
        self.values(node)
            .find_map(|value| value.get("value").and_then(Value::as_str))
    }

    /// Each `CLUSTER` `node`, read the same way, such as a system client's
    /// facilities (`id10`).
    #[must_use]
    pub fn clusters(self, node: &'a str) -> Vec<Credentials<'a>> {
        self.items(node)
            .map(|item| Credentials(Some(item)))
            .collect()
    }
}

/// What one caller may do: the system `AGENT` its client identifier names,
/// and the grants in force on the request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Capabilities {
    agent: Option<Agent>,
    grants: Vec<Grant>,
}

impl Capabilities {
    /// The capabilities of the system `AGENT` `agent` or of a party, holding
    /// `grants`.
    pub(crate) fn new(agent: Option<Agent>, grants: Vec<Grant>) -> Self {
        Self { agent, grants }
    }

    /// The capabilities of the caller of the running request; none outside
    /// a request, for a caller who is no system client and binds to no party,
    /// and for one the policy parties grant nothing.
    #[must_use]
    pub fn current() -> Arc<Self> {
        CURRENT.try_with(Arc::clone).unwrap_or_default()
    }

    /// The system `AGENT` the caller's client identifier names; `None` for
    /// a party, or a client no `AGENT` carries.
    #[must_use]
    pub fn agent(&self) -> Option<Agent> {
        self.agent
    }

    /// Every grant in force.
    #[must_use]
    pub fn grants(&self) -> &[Grant] {
        &self.grants
    }

    /// The grants in force of `kind`.
    pub fn of_kind(&self, kind: GrantKind) -> impl Iterator<Item = &Grant> {
        self.grants.iter().filter(move |grant| grant.kind == kind)
    }

    /// Whether a grant of `kind` is in force.
    #[must_use]
    pub fn holds(&self, kind: GrantKind) -> bool {
        self.of_kind(kind).next().is_some()
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

    #[test]
    fn credentials_read_by_node_and_through_a_cluster() {
        let tree = json!({"_type": "ITEM_TREE", "archetype_node_id": "id8", "items": [
            {"archetype_node_id": "id9", "value": {"_type": "DV_TEXT", "value": "T.v1"}},
            {"archetype_node_id": "id9.1", "value": {"_type": "DV_CODED_TEXT", "value": "x",
                "defining_code": {"code_string": "at20"}}},
            {"archetype_node_id": "id10", "items": [
                {"archetype_node_id": "id11", "value": {"_type": "DV_TEXT", "value": "urn:x"}}
            ]},
            {"archetype_node_id": "id12", "value": {"_type": "DV_COUNT", "magnitude": 50}},
            {"archetype_node_id": "id21", "value": {"_type": "DV_BOOLEAN", "value": true}}
        ]});
        let grant = Grant::new(GrantKind::DataMigration, Uuid::nil(), Some(Arc::new(tree)));
        let credentials = grant.credentials();
        assert_eq!(credentials.texts("id9"), vec!["T.v1", "x"]);
        assert_eq!(credentials.codes("id9"), vec!["at20"]);
        assert_eq!(credentials.count("id12"), Some(50));
        assert_eq!(credentials.boolean("id21"), Some(true));
        let [facility] = credentials.clusters("id10")[..] else {
            panic!("one facility");
        };
        assert_eq!(facility.texts("id11"), vec!["urn:x"]);
        assert_eq!(
            Grant::new(GrantKind::Import, Uuid::nil(), None)
                .credentials()
                .texts("id9"),
            Vec::<&str>::new()
        );
    }

    #[tokio::test]
    async fn the_current_capabilities_are_the_requests() {
        assert_eq!(*Capabilities::current(), Capabilities::default());
        let granted = Arc::new(Capabilities::new(
            None,
            vec![Grant::new(GrantKind::SubjectAudit, Uuid::nil(), None)],
        ));
        let held = with_capabilities(Arc::clone(&granted), async {
            Capabilities::current().holds(GrantKind::SubjectAudit)
        })
        .await;
        assert!(held);
    }
}
