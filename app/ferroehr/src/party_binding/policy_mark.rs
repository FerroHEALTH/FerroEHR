// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy parties, recognised by what their versions hold, and the mark
//! that keeps one hidden from every caller but an administrator.
//!
//! A version is a policy party when it holds, in any archetype or template:
//!
//! - a `CAPABILITY` whose `credentials` hold, anywhere in their tree, a
//!   `DV_CODED_TEXT` of the draft access terminology ([`TERMINOLOGY`]);
//! - a `PARTY_RELATIONSHIP`, at its root or inline, whose `name` or whose
//!   `details` hold the acts-for code ([`ACTS_FOR`]) of that terminology;
//! - an `ELEMENT` named by the client identifier code ([`CLIENT_ID`]) of that
//!   terminology and holding a `DV_IDENTIFIER`, which binds an actor to an
//!   OAuth 2.0 client (`client_identifier`). A `DV_IDENTIFIER` merely typed
//!   `client_id` is the deployment's own data and marks nothing.
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

use std::borrow::Cow;
use std::fmt;

use serde_json::Value;
use sqlx::PgConnection;

use crate::ids::VoId;
use crate::storage::error::StorageError;
use crate::versioning::tree::{Shape, Tree};
use openehr_rm::draft::access_control::terminology::{TERMINOLOGY_ID_ACCESS, Term};

/// The `TERMINOLOGY_ID` the operation grants, their qualifiers and the
/// relationship codes are coded in: the draft `openehr_access` terminology.
pub const TERMINOLOGY: &str = TERMINOLOGY_ID_ACCESS;

/// The code of [`TERMINOLOGY`] that makes a `PARTY_RELATIONSHIP` an acts-for
/// relationship.
pub const ACTS_FOR: &str = "acts_for";

/// The code of [`TERMINOLOGY`] that makes a `PARTY_RELATIONSHIP` a
/// representation: its source acts for its target, the subject, on the
/// subject's own record. It makes no policy party.
pub const REPRESENTS: &str = "represents";

/// The code of [`TERMINOLOGY`] that names the `ELEMENT` holding an actor's
/// OAuth 2.0 `client_id`.
pub const CLIENT_ID: &str = "client_id";

/// The SQL/JSON path that holds exactly when `policy_content` finds content.
pub const POLICY_PATH: &str = "$ ? (\
exists($.**?(@._type == \"CAPABILITY\").credentials.**?(@._type == \"DV_CODED_TEXT\" \
&& @.defining_code.terminology_id.value == \"openehr_access\")) \
|| exists($.**?(@._type == \"PARTY_RELATIONSHIP\")?(\
exists(@.name?(@._type == \"DV_CODED_TEXT\" \
&& @.defining_code.terminology_id.value == \"openehr_access\" \
&& @.defining_code.code_string == \"acts_for\")) \
|| exists(@.details.**?(@._type == \"DV_CODED_TEXT\" \
&& @.defining_code.terminology_id.value == \"openehr_access\" \
&& @.defining_code.code_string == \"acts_for\")))) \
|| exists($.**?(@._type == \"ELEMENT\" && @.value._type == \"DV_IDENTIFIER\" \
&& @.name._type == \"DV_CODED_TEXT\" \
&& @.name.defining_code.terminology_id.value == \"openehr_access\" \
&& @.name.defining_code.code_string == \"client_id\")))";

/// What makes a version a policy party.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyContent {
    /// A capability's credentials hold this code of [`TERMINOLOGY`].
    Grant(String),
    /// A `PARTY_RELATIONSHIP` carries the acts-for code.
    ActsFor,
    /// An `ELEMENT` coded [`CLIENT_ID`] names an OAuth 2.0 client.
    Client,
}

impl fmt::Display for PolicyContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Grant(code) => write!(f, "a capability granting {TERMINOLOGY}::{code}"),
            Self::ActsFor => write!(f, "an acts-for relationship ({TERMINOLOGY}::{ACTS_FOR})"),
            Self::Client => write!(f, "a client identifier ({TERMINOLOGY}::{CLIENT_ID})"),
        }
    }
}

/// The text member `key` of `value`.
fn text<'a, T: Tree<'a>>(value: T, key: &str) -> Option<Cow<'a, str>> {
    value.get_text(key)
}

/// The terminology id and code of `value` when it is a `DV_CODED_TEXT`.
fn phrase<'a, T: Tree<'a>>(value: T) -> Option<(Cow<'a, str>, Cow<'a, str>)> {
    let code = value.get("defining_code")?;
    let terminology = code
        .get("terminology_id")
        .and_then(|id| text(id, "value"))?;
    (text(value, "_type").as_deref() == Some("DV_CODED_TEXT"))
        .then(|| Some((terminology, text(code, "code_string")?)))
        .flatten()
}

/// The code of `value` when it is a `DV_CODED_TEXT` of [`TERMINOLOGY`].
pub(crate) fn coded<'a, T: Tree<'a>>(value: T) -> Option<Cow<'a, str>> {
    phrase(value)
        .filter(|(terminology, _)| terminology == TERMINOLOGY)
        .map(|(_, code)| code)
}

/// The `DV_CODED_TEXT` of the access term `term`, reading `rubric`: the one
/// place a written code names its terminology.
pub(crate) fn coded_term<T: Term>(term: T, rubric: &str) -> Value {
    openehr_its::json::to_canonical_value(&openehr_rm::prelude::DvCodedText {
        value: rubric.to_owned(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
        defining_code: openehr_rm::prelude::CodePhrase {
            terminology_id: openehr_base::prelude::TerminologyId {
                value: term.terminology().to_owned(),
            },
            code_string: term.code().to_owned(),
            preferred_term: None,
        },
    })
}

/// The first value of the tree `value`, itself included, depth first, for
/// which `found` answers.
pub(crate) fn first<'a, T: Tree<'a>, R>(
    value: T,
    found: &mut impl FnMut(T) -> Option<R>,
) -> Option<R> {
    if let Some(hit) = found(value) {
        return Some(hit);
    }
    match value.shape() {
        Shape::Object => value.members().find_map(|(_, child)| first(child, found)),
        Shape::Array => value.items().find_map(|child| first(child, found)),
        Shape::String | Shape::Other => None,
    }
}

/// Whether the `PARTY_RELATIONSHIP` `relationship` carries the code `wanted`
/// of `terminology`, such as [`ACTS_FOR`] or [`REPRESENTS`] of [`TERMINOLOGY`],
/// in its `name` or anywhere in its `details`.
pub(crate) fn carries<'a, T: Tree<'a>>(relationship: T, terminology: &str, wanted: &str) -> bool {
    let code = |item: T| {
        phrase(item)
            .filter(|(of, code)| of == terminology && code == wanted)
            .map(drop)
    };
    relationship.get("name").and_then(code).is_some()
        || relationship
            .get("details")
            .and_then(|details| first(details, &mut |item| code(item)))
            .is_some()
}

/// The openEHR `subject_relationship` code `relationship` names: the first
/// `DV_CODED_TEXT` of the `openehr` terminology in its `name` or its
/// `details` whose code is of that group, `None` when it names none.
pub(crate) fn subject_relationship<'a, T: Tree<'a>>(relationship: T) -> Option<String> {
    relationship
        .get("name")
        .and_then(openehr_relationship)
        .or_else(|| first(relationship.get("details")?, &mut openehr_relationship))
}

/// The code of `item` when it is a `DV_CODED_TEXT` of the `openehr`
/// terminology's `subject_relationship` group.
fn openehr_relationship<'a, T: Tree<'a>>(item: T) -> Option<String> {
    let defining = item.get("defining_code")?;
    let of = defining
        .get("terminology_id")
        .and_then(|id| text(id, "value"));
    let code = text(defining, "code_string")?;
    (text(item, "_type").as_deref() == Some("DV_CODED_TEXT")
        && of.as_deref() == Some("openehr")
        && openehr_term::bundle::openehr().is_valid_code("subject_relationship", &code))
    .then(|| code.into_owned())
}

/// The `(issuer, id)` of `value` when it is a client identifier: an `ELEMENT`
/// whose `name` is [`TERMINOLOGY`]`::`[`CLIENT_ID`], holding a
/// `DV_IDENTIFIER` with an `issuer`, whatever its `type`; the one rule the
/// policy mark and the client rows read (#6542).
pub(crate) fn client_identifier<'a, T: Tree<'a>>(value: T) -> Option<(String, String)> {
    let identifier = value.get("value")?;
    (text(value, "_type").as_deref() == Some("ELEMENT")
        && value.get("name").and_then(coded).as_deref() == Some(CLIENT_ID)
        && text(identifier, "_type").as_deref() == Some("DV_IDENTIFIER"))
    .then(|| {
        Some((
            text(identifier, "issuer")?.into_owned(),
            text(identifier, "id")?.into_owned(),
        ))
    })
    .flatten()
}

/// What makes the party body `body`, in canonical JSON, a policy party, or
/// `None` when it holds none of it.
#[must_use]
pub(crate) fn policy_content<'a, T: Tree<'a>>(body: T) -> Option<PolicyContent> {
    first(body, &mut |value| match text(value, "_type")?.as_ref() {
        "CAPABILITY" => first(value.get("credentials")?, &mut |item| {
            coded(item).map(Cow::into_owned)
        })
        .map(PolicyContent::Grant),
        "PARTY_RELATIONSHIP" => {
            carries(value, TERMINOLOGY, ACTS_FOR).then_some(PolicyContent::ActsFor)
        }
        "ELEMENT" => client_identifier(value).map(|_| PolicyContent::Client),
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
    fn the_operation_terminology_is_the_draft_access_terminology() {
        // The grants and the draft's access groups share one terminology, so
        // the draft invariants and the policy marks agree.
        assert_eq!(TERMINOLOGY, "openehr_access");
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

    /// An `ELEMENT` named [`CLIENT_ID`] of [`TERMINOLOGY`], holding `value`.
    fn client_element(value: &Value) -> Value {
        json!({ "_type": "ELEMENT", "archetype_node_id": "at1",
            "name": coded_text(CLIENT_ID, TERMINOLOGY), "value": value })
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
                            "items": [client_element(&json!({ "_type": "DV_IDENTIFIER",
                                "issuer": "https://login.example", "id": "lab", "type": "system" }))] } }] }),
                Some(PolicyContent::Client),
            ),
            (
                json!({ "_type": "AGENT", "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-AGENT.gateway.v2",
                    "details": { "_type": "ITEM_TREE", "archetype_node_id": "at9",
                        "items": [element(&json!({ "_type": "DV_IDENTIFIER",
                            "issuer": "https://login.example", "id": "lab", "type": "client_id" }))] } }),
                None,
            ),
            (
                json!({ "_type": "ORGANISATION", "archetype_node_id": "x",
                    "details": { "_type": "ITEM_TREE", "archetype_node_id": "at9",
                        "items": [client_element(&json!({ "_type": "DV_TEXT", "value": "lab" }))] } }),
                None,
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
        assert!(carries(&representation, TERMINOLOGY, REPRESENTS));
        assert!(!carries(&representation, TERMINOLOGY, ACTS_FOR));
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
}
