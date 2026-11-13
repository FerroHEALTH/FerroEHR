// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy parties, recognised by what their versions hold, and the mark
//! that keeps one hidden from every caller but an administrator.
//!
//! A version is a policy party when it holds, in any archetype or template:
//!
//! - a `CAPABILITY` whose `credentials` hold, anywhere in their tree, a
//!   `DV_CODED_TEXT` of FerroEHR's operation terminology ([`TERMINOLOGY`]);
//! - a `PARTY_RELATIONSHIP`, at its root or inline, whose `name` or whose
//!   `details` hold the acts-for code ([`ACTS_FOR`]) of that terminology;
//! - a `DV_IDENTIFIER` of the client identifier type ([`CLIENT_ID`]), which
//!   binds an actor to an OAuth 2.0 client.
//!
//! A party with such a version is marked in `vo_head.policy_party_at`: the
//! commit of the version sets it in its own transaction
//! (`party_binding::protect::check`), the party migration and
//! `mark_stored` set it from the stored versions, and nothing clears it.
//! Only an administrator writes or reads a marked party, through the
//! Demographic API and through demographic AQL alike, every version of it.
//! No openEHR spec gives a capability an authorization meaning or governs who
//! reads a party: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the canonical party body a commit \
              holds, walked for the coded content that makes it a policy party"
)]

use std::fmt;

use serde_json::Value;
use sqlx::PgConnection;

use crate::ids::VoId;
use crate::storage::error::StorageError;

/// The `TERMINOLOGY_ID` of FerroEHR's operation terminology.
pub const TERMINOLOGY: &str = "ferroehr-operations";

/// The code of [`TERMINOLOGY`] that makes a `PARTY_RELATIONSHIP` an acts-for
/// relationship.
pub const ACTS_FOR: &str = "acts_for";

/// The code of [`TERMINOLOGY`] that makes a `PARTY_RELATIONSHIP` a
/// representation: its source acts for its target, the subject, on the
/// subject's own record. It makes no policy party.
pub const REPRESENTS: &str = "represents";

/// The `DV_IDENTIFIER.type` of an OAuth 2.0 `client_id`.
pub const CLIENT_ID: &str = "client_id";

/// The SQL/JSON path that holds exactly when [`policy_content`] finds content.
///
/// The party migration `0034_policy_party_mark.sql` and `mark_stored` test
/// each version's body with it, and a test runs both over one set of bodies.
pub const POLICY_PATH: &str = "$ ? (\
exists($.**?(@._type == \"CAPABILITY\").credentials.**?(@._type == \"DV_CODED_TEXT\" \
&& @.defining_code.terminology_id.value == \"ferroehr-operations\")) \
|| exists($.**?(@._type == \"PARTY_RELATIONSHIP\")?(\
exists(@.name?(@._type == \"DV_CODED_TEXT\" \
&& @.defining_code.terminology_id.value == \"ferroehr-operations\" \
&& @.defining_code.code_string == \"acts_for\")) \
|| exists(@.details.**?(@._type == \"DV_CODED_TEXT\" \
&& @.defining_code.terminology_id.value == \"ferroehr-operations\" \
&& @.defining_code.code_string == \"acts_for\")))) \
|| exists($.**?(@._type == \"DV_IDENTIFIER\" && @.type == \"client_id\")))";

/// What makes a version a policy party.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyContent {
    /// A capability's credentials hold this code of [`TERMINOLOGY`].
    Grant(String),
    /// A `PARTY_RELATIONSHIP` carries the acts-for code.
    ActsFor,
    /// A `DV_IDENTIFIER` names an OAuth 2.0 client.
    Client,
}

impl fmt::Display for PolicyContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Grant(code) => write!(f, "a capability granting {TERMINOLOGY}::{code}"),
            Self::ActsFor => write!(f, "an acts-for relationship ({TERMINOLOGY}::{ACTS_FOR})"),
            Self::Client => write!(f, "a {CLIENT_ID} identifier"),
        }
    }
}

/// The text member `key` of `value`.
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// The code of `value` when it is a `DV_CODED_TEXT` of [`TERMINOLOGY`].
pub(crate) fn coded(value: &Value) -> Option<&str> {
    let code = value.get("defining_code")?;
    let terminology = code.get("terminology_id").and_then(|id| text(id, "value"));
    (text(value, "_type") == Some("DV_CODED_TEXT") && terminology == Some(TERMINOLOGY))
        .then(|| text(code, "code_string"))
        .flatten()
}

/// The first value of the tree `value`, itself included, depth first, for
/// which `found` answers.
fn first<T>(value: &Value, found: &mut impl FnMut(&Value) -> Option<T>) -> Option<T> {
    if let Some(hit) = found(value) {
        return Some(hit);
    }
    match value {
        Value::Object(members) => members.values().find_map(|child| first(child, found)),
        Value::Array(items) => items.iter().find_map(|child| first(child, found)),
        _ => None,
    }
}

/// Whether the `PARTY_RELATIONSHIP` `relationship` carries `wanted`, a code of
/// [`TERMINOLOGY`] such as [`ACTS_FOR`] or [`REPRESENTS`], in its `name` or
/// anywhere in its `details`.
pub(crate) fn carries(relationship: &Value, wanted: &str) -> bool {
    let code = |item: &Value| (coded(item) == Some(wanted)).then_some(());
    relationship.get("name").and_then(code).is_some()
        || relationship
            .get("details")
            .and_then(|details| first(details, &mut |item| code(item)))
            .is_some()
}

/// The openEHR `subject_relationship` code `relationship` names: the first
/// `DV_CODED_TEXT` of the `openehr` terminology in its `name` or its
/// `details` whose code is of that group, `None` when it names none.
pub(crate) fn subject_relationship(relationship: &Value) -> Option<String> {
    relationship
        .get("name")
        .and_then(openehr_relationship)
        .or_else(|| first(relationship.get("details")?, &mut openehr_relationship))
}

/// The code of `item` when it is a `DV_CODED_TEXT` of the `openehr`
/// terminology's `subject_relationship` group.
fn openehr_relationship(item: &Value) -> Option<String> {
    let defining = item.get("defining_code")?;
    let of = defining
        .get("terminology_id")
        .and_then(|id| text(id, "value"));
    let code = text(defining, "code_string")?;
    (text(item, "_type") == Some("DV_CODED_TEXT")
        && of == Some("openehr")
        && openehr_term::bundle::openehr().is_valid_code("subject_relationship", code))
    .then(|| code.to_owned())
}

/// What makes the party body `body`, in canonical JSON, a policy party, or
/// `None` when it holds none of it.
#[must_use]
pub fn policy_content(body: &Value) -> Option<PolicyContent> {
    first(body, &mut |value| match text(value, "_type")? {
        "CAPABILITY" => first(value.get("credentials")?, &mut |item| {
            coded(item).map(str::to_owned)
        })
        .map(PolicyContent::Grant),
        "PARTY_RELATIONSHIP" => carries(value, ACTS_FOR).then_some(PolicyContent::ActsFor),
        "DV_IDENTIFIER" => {
            (text(value, "type") == Some(CLIENT_ID)).then_some(PolicyContent::Client)
        }
        _ => None,
    })
}

/// Marks the party `vo_id` (every party when `None`) when one of its stored
/// versions holds [`policy_content`], tested by [`POLICY_PATH`], on `conn`:
/// what an import, an archive load and a refill do after the versions land.
/// A mark is never cleared.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn mark_stored(
    conn: &mut PgConnection,
    vo_id: Option<VoId>,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE vo_head h SET policy_party_at = now() \
         WHERE h.policy_party_at IS NULL AND ($1::uuid IS NULL OR h.vo_id = $1) \
           AND EXISTS (SELECT FROM version v \
                       WHERE v.vo_id = h.vo_id AND v.body IS NOT NULL \
                         AND jsonb_path_exists(v.body::jsonb, $2::jsonpath))",
    )
    .bind(vo_id)
    .bind(POLICY_PATH)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Marks the party `vo_id`, whose committed version holds
/// [`policy_content`], on its commit transaction `tx`; a mark it already
/// holds is kept.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn mark(tx: &mut PgConnection, vo_id: VoId) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE vo_head SET policy_party_at = now() \
         WHERE vo_id = $1 AND policy_party_at IS NULL",
    )
    .bind(vo_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Whether the party `vo_id` carries the mark, read by the head's primary
/// key on `conn`; `false` for a party with no head row.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn marked(conn: &mut PgConnection, vo_id: VoId) -> Result<bool, StorageError> {
    let marked: Option<bool> =
        sqlx::query_scalar("SELECT policy_party_at IS NOT NULL FROM vo_head WHERE vo_id = $1")
            .bind(vo_id)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(marked == Some(true))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_draft_access_control_terminology_is_the_operation_terminology() {
        // The draft access control's groups live in this terminology until
        // openEHR adopts them, so its invariants and the policy marks agree.
        assert_eq!(
            TERMINOLOGY,
            openehr_rm::draft::access_control::invariants::TERMINOLOGY_ID_ACCESS_CONTROL
        );
    }

    /// A `DV_CODED_TEXT` of `code` in `terminology`.
    fn coded_text(code: &str, terminology: &str) -> Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
    }

    /// An `ELEMENT` holding `value`.
    fn element(value: &Value) -> Value {
        json!({ "_type": "ELEMENT", "archetype_node_id": "at1",
            "name": { "_type": "DV_TEXT", "value": "item" }, "value": value })
    }

    /// A `ROLE` of a deployment's own archetype with one capability whose
    /// credentials hold `items` in a cluster.
    fn role(items: &Value) -> Value {
        json!({ "_type": "ROLE", "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-ROLE.legacy_migration.v3",
            "capabilities": [{ "_type": "CAPABILITY", "archetype_node_id": "at2",
                "name": { "_type": "DV_TEXT", "value": "Migration" },
                "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at3",
                    "items": [{ "_type": "CLUSTER", "archetype_node_id": "at4", "items": items }] } }] })
    }

    /// A body holding `relationship` inline.
    fn holding(relationship: &Value) -> Value {
        json!({ "_type": "ORGANISATION", "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.mine.v2",
            "relationships": [relationship] })
    }

    /// The bodies the content test and [`POLICY_PATH`] are both run over,
    /// each with what the content test finds in it.
    fn cases() -> Vec<(Value, Option<PolicyContent>)> {
        let grant = PolicyContent::Grant("import_ehr".to_owned());
        vec![
            (
                role(&json!([element(&coded_text("import_ehr", TERMINOLOGY))])),
                Some(grant),
            ),
            (
                role(&json!([element(&coded_text("not_after", TERMINOLOGY))])),
                Some(PolicyContent::Grant("not_after".to_owned())),
            ),
            (
                role(&json!([element(&coded_text("import_ehr", "local"))])),
                None,
            ),
            (
                role(&json!([element(&coded_text(
                    "import_ehr",
                    "FERROEHR-OPERATIONS"
                ))])),
                None,
            ),
            (
                json!({ "_type": "ROLE", "archetype_node_id": "x",
                    "name": coded_text("import_ehr", TERMINOLOGY), "capabilities": [] }),
                None,
            ),
            (
                holding(
                    &json!({ "_type": "PARTY_RELATIONSHIP", "archetype_node_id": "at5",
                    "name": coded_text(ACTS_FOR, TERMINOLOGY) }),
                ),
                Some(PolicyContent::ActsFor),
            ),
            (
                json!({ "_type": "PARTY_RELATIONSHIP", "archetype_node_id": "org.example::rel.v1",
                    "name": { "_type": "DV_TEXT", "value": "on behalf of" },
                    "details": { "_type": "ITEM_TREE", "archetype_node_id": "at6",
                        "items": [element(&coded_text(ACTS_FOR, TERMINOLOGY))] } }),
                Some(PolicyContent::ActsFor),
            ),
            (
                holding(
                    &json!({ "_type": "PARTY_RELATIONSHIP", "archetype_node_id": "at5",
                    "name": coded_text("import_ehr", TERMINOLOGY) }),
                ),
                None,
            ),
            (
                json!({ "_type": "ORGANISATION", "archetype_node_id": "x",
                    "details": { "_type": "ITEM_TREE", "archetype_node_id": "at7",
                        "items": [element(&coded_text(ACTS_FOR, TERMINOLOGY))] } }),
                None,
            ),
            (
                json!({ "_type": "AGENT", "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-AGENT.gateway.v2",
                    "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at8",
                        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at9",
                            "items": [element(&json!({ "_type": "DV_IDENTIFIER",
                                "issuer": "https://login.example", "id": "lab", "type": CLIENT_ID }))] } }] }),
                Some(PolicyContent::Client),
            ),
            (
                json!({ "_type": "PERSON", "archetype_node_id": "x",
                    "details": { "_type": "ITEM_TREE", "archetype_node_id": "at9",
                        "items": [element(&json!({ "_type": "DV_IDENTIFIER",
                            "issuer": "NL", "id": "123", "type": "BSN" }))] } }),
                None,
            ),
        ]
    }

    /// A relationship names the openEHR `subject_relationship` code of its
    /// name or its details, and no code outside that group: the rule that
    /// held a mapped relationship type to the group.
    #[test]
    fn a_relationship_names_only_a_subject_relationship_code() {
        let named = |name: &Value, items: &Value| {
            subject_relationship(&json!({ "_type": "PARTY_RELATIONSHIP",
                "archetype_node_id": "org.example::rel.v1", "name": name,
                "details": { "_type": "ITEM_TREE", "archetype_node_id": "at6", "items": items } }))
        };
        let label = json!({ "_type": "DV_TEXT", "value": "guardian" });
        assert_eq!(
            named(&coded_text("264", "openehr"), &json!([])).as_deref(),
            Some("264")
        );
        assert_eq!(
            named(&label, &json!([element(&coded_text("254", "openehr"))])).as_deref(),
            Some("254")
        );
        for outside in [coded_text("9999", "openehr"), coded_text("264", "local")] {
            assert_eq!(
                named(&label, &json!([element(&outside)])),
                None,
                "{outside}"
            );
        }
        let representation = json!({ "_type": "PARTY_RELATIONSHIP",
            "archetype_node_id": "at5", "name": coded_text(REPRESENTS, TERMINOLOGY) });
        assert!(carries(&representation, REPRESENTS));
        assert!(!carries(&representation, ACTS_FOR));
        assert_eq!(
            policy_content(&holding(&json!({ "_type": "PARTY_RELATIONSHIP",
                "archetype_node_id": "at5", "name": coded_text(REPRESENTS, TERMINOLOGY) }))),
            None,
            "a representation makes no policy party"
        );
    }

    #[test]
    fn the_coded_content_makes_a_policy_party_in_any_archetype() {
        for (body, expected) in cases() {
            assert_eq!(policy_content(&body), expected, "{body}");
        }
    }

    /// PostgreSQL 18 answers [`POLICY_PATH`] over each body as the content
    /// test does, so a stored body is marked exactly when a commit of it
    /// would be.
    #[tokio::test]
    async fn the_path_agrees_with_the_content_test() {
        let db = testkit::db().await.expect("testkit database");
        for (body, expected) in cases() {
            let held: bool =
                sqlx::query_scalar("SELECT jsonb_path_exists($1::jsonb, $2::jsonpath)")
                    .bind(body.to_string())
                    .bind(POLICY_PATH)
                    .fetch_one(&db.pool())
                    .await
                    .expect("the path runs");
            assert_eq!(held, expected.is_some(), "{body}");
        }
    }

    /// The party migration that adds the mark tests the stored bodies with
    /// [`POLICY_PATH`] as [`mark_stored`] does.
    #[test]
    fn the_migration_marks_by_the_same_path() {
        let migration = include_str!("../../migrations/party/0034_policy_party_mark.sql");
        assert!(
            migration.contains(&format!(
                "jsonb_path_exists(v.body::jsonb, '{POLICY_PATH}')"
            )),
            "the migration's path:\n'{POLICY_PATH}'"
        );
        assert!(
            migration.contains("WHERE v.vo_id = h.vo_id AND v.body IS NOT NULL"),
            "the migration's join"
        );
    }
}
