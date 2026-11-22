// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The record subjects: the EHR subject references a patient's `PERSON`
//! stands for.
//!
//! They link an EHR whose `EHR_STATUS` names a pseudonym to the `PERSON`
//! whose care relationships and representations it inherits.
//!
//! A `PERSON` declares one by an `ELEMENT` anywhere in its `PARTY.details`
//! named by the code [`RecordSubject::RecordSubject`] of [`TERMINOLOGY_ID_ACCESS`],
//! whose value is a `DV_IDENTIFIER`: `issuer` is the reference's namespace
//! and `id` its id. An identifier another organisation issued, such as an
//! identity source's pseudonym, is recorded in `details` and never in
//! `identities` (RM Demographic IM §2.1.3), and the cross-reference stays out
//! of the EHR (RM EHR IM §EHR Status: "the change will be made elsewhere in a
//! cross-reference table"). The registering system writes the element in
//! the same demographic `CONTRIBUTION` as the `PERSON`, and an identity merge
//! moves it from one `PERSON` to another in one contribution.
//!
//! The rows are projected into `binding_record_subject` in each commit's
//! transaction beside the caller facts, stamp the `PERSON` so every replica
//! reads them again ([`super::store`]), and are held in memory by
//! [`crate::party_binding::grants::live::LiveGrants`]. The element, its
//! projection, the uniqueness of a reference and the order in which a
//! reference resolves are governed by no openEHR spec: our own design.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::refusal::{Clash, Refusal};
use crate::error::{ServiceError, Violation};
use crate::party_binding::derive::Facts;
use crate::party_binding::grants::derive::{GrantFacts, walk};
use crate::party_binding::grants::store::GrantHead;
use crate::party_binding::policy_mark::coded;
use crate::party_binding::step::{Step, Tag, differ};
use crate::privacy::PrivacyPolicy;
use crate::storage::access_projection::EhrSubject;
use crate::storage::error::StorageError;
use crate::versioning::Kind;
use crate::versioning::tree::Tree;
use openehr_rm::draft::access_control::terminology::{RecordSubject, TERMINOLOGY_ID_ACCESS, Term};

/// The unique constraint of `binding_record_subject` (party/0041).
const UNIQUE: &str = "uq_binding_record_subject";

/// One record subject a `PERSON` body declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeclaredSubject {
    /// The element's position among the record subject elements of the
    /// body's `details`, depth first.
    pub(crate) ordinal: i32,
    /// The reference it names.
    pub(crate) subject: EhrSubject,
}

/// Whether `node` is an element named by the record subject code.
fn names_record_subject<'a, T: Tree<'a>>(node: T) -> bool {
    node.get("name")
        .and_then(coded)
        .is_some_and(|code| RecordSubject::read(TERMINOLOGY_ID_ACCESS, &code).is_ok())
}

/// Pushes onto `facts` the record subjects `body`, a party of `kind`,
/// declares in its `details`, and why one cannot be read: an element whose
/// value is no `DV_IDENTIFIER` with an issuer and an id, and one held by a
/// party other than a `PERSON` or outside its `details`.
pub(crate) fn declared<'a, T: Tree<'a>>(kind: Kind, body: T, facts: &mut GrantFacts) {
    let code = RecordSubject::RecordSubject.code();
    let mut in_details = 0_usize;
    if kind == Kind::Person
        && let Some(details) = body.get("details")
    {
        walk(details, &mut |node| {
            if !names_record_subject(node) {
                return;
            }
            let ordinal = i32::try_from(in_details).unwrap_or(i32::MAX);
            in_details += 1;
            match identifier(node.get("value")) {
                Some(subject) => {
                    if !facts.subjects.iter().any(|held| held.subject == subject) {
                        facts.subjects.push(DeclaredSubject { ordinal, subject });
                    }
                }
                None => facts.unread.push(format!(
                    "details: a {TERMINOLOGY_ID_ACCESS}::{code} element holds no DV_IDENTIFIER with an \
                     issuer and an id"
                )),
            }
        });
    }
    let mut everywhere = 0_usize;
    walk(body, &mut |node| {
        if names_record_subject(node) {
            everywhere += 1;
        }
    });
    if everywhere > in_details {
        facts.unread.push(format!(
            "a {TERMINOLOGY_ID_ACCESS}::{code} element is held only in the details of a PERSON, never in \
             another kind of party or outside PARTY.details (RM Demographic IM §2.1.3)"
        ));
    }
}

/// The reference the value `value` of a record subject element names: a
/// `DV_IDENTIFIER` with a non-empty `issuer` and `id`.
fn identifier<'a, T: Tree<'a>>(value: Option<T>) -> Option<EhrSubject> {
    let value = value?;
    let text = |key: &str| value.get_text(key).filter(|text| !text.trim().is_empty());
    (text("_type").as_deref() == Some("DV_IDENTIFIER"))
        .then(|| Some(EhrSubject::new(text("issuer")?, text("id")?)))
        .flatten()
}

/// Refuses the committed body `facts` when a record subject it declares
/// names this CDR's own demographic namespace `own`, whose references name a
/// `PERSON` by its container, or a declared pseudonym namespace of
/// `privacy` with an id its subject rule refuses, so the party side holds a
/// reference the `EHR_STATUS` side would refuse.
///
/// # Errors
/// [`ServiceError::Unprocessable`] (`422`) naming the first such reference's
/// namespace, never its id.
pub(crate) fn check(
    facts: Option<&Facts>,
    own: &str,
    privacy: &PrivacyPolicy,
) -> Result<(), ServiceError> {
    let code = RecordSubject::RecordSubject.code();
    for declared in facts.map_or(&[][..], |facts| facts.grants.subjects.as_slice()) {
        let namespace = &declared.subject.namespace;
        let why = if namespace == own {
            "is this CDR's own demographic namespace, whose subject references name a PERSON \
             by its container"
        } else if privacy.subject_namespaces().contains(namespace)
            && !privacy.admits_subject(namespace, &declared.subject.id)
        {
            "is a declared [privacy] subject namespace, whose subject rule refuses this id"
        } else {
            continue;
        };
        return Err(ServiceError::content_invalid(Violation::new(format!(
            "a {TERMINOLOGY_ID_ACCESS}::{code} element cannot be read: its issuer {namespace:?} {why}"
        ))));
    }
    Ok(())
}

/// Replaces the record subjects of the `PERSON` `head` with the ones its
/// live body declares, on its commit transaction `tx`; whether any went or
/// came.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn replace(
    tx: &mut PgConnection,
    head: GrantHead<'_>,
) -> Result<bool, StorageError> {
    let (gone, came) = steps(head);
    let mut rows = gone.run(tx).await?;
    if head
        .live()
        .is_some_and(|facts| !facts.grants.subjects.is_empty())
    {
        rows.extend(came.run(tx).await?);
    }
    Ok(differ(&rows, Tag::SubjectsGone, Tag::SubjectsCame))
}

/// The two steps that replace the record subjects of `head`: the delete of
/// those it declared and the insert of those its live body declares, each
/// returning the text of every row it wrote ([`Tag::SubjectsGone`],
/// [`Tag::SubjectsCame`]).
pub(crate) fn steps(head: GrantHead<'_>) -> (Step<'_>, Step<'_>) {
    let holder = head.vo_id;
    let gone = Step::new(
        "sd",
        "DELETE FROM binding_record_subject WHERE holder_id = $1 \
         RETURNING holder_id AS p, ROW(namespace, subject_id, lifecycle_state)::text AS t",
        Some(Tag::SubjectsGone),
        move |query| query.bind(holder),
    );
    let declared = head
        .live()
        .map_or(&[][..], |facts| facts.grants.subjects.as_slice());
    let ordinal: Vec<i32> = declared.iter().map(|d| d.ordinal).collect();
    let namespace: Vec<&str> = declared
        .iter()
        .map(|d| d.subject.namespace.as_str())
        .collect();
    let id: Vec<&str> = declared.iter().map(|d| d.subject.id.as_str()).collect();
    let came = Step::new(
        "si",
        "INSERT INTO binding_record_subject \
             (holder_id, ordinal, namespace, subject_id, lifecycle_state) \
         SELECT $1, t.ordinal, t.namespace, t.subject_id, $2 \
         FROM unnest($3::integer[], $4::text[], $5::text[]) \
              AS t(ordinal, namespace, subject_id) \
         RETURNING holder_id AS p, ROW(namespace, subject_id, lifecycle_state)::text AS t",
        Some(Tag::SubjectsCame),
        move |query| {
            query
                .bind(holder)
                .bind(head.lifecycle_state)
                .bind(ordinal)
                .bind(namespace)
                .bind(id)
        },
    );
    (gone, came)
}

/// Checks the deferred uniqueness of the record subjects at the end of a
/// commit of versions of `kinds` on `tx`, when one is a `PERSON`, so a
/// reference two `PERSON`s declare refuses the commit while the versions of
/// a contribution may move a reference in any order.
///
/// # Errors
/// [`ServiceError::Conflict`] (`409`) when another `PERSON` declares a
/// reference this commit declares, and a storage failure.
pub(crate) async fn settle(
    tx: &mut PgConnection,
    kinds: impl IntoIterator<Item = Kind>,
) -> Result<(), ServiceError> {
    if !kinds.into_iter().any(|kind| kind == Kind::Person) {
        return Ok(());
    }
    let checked = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "SET CONSTRAINTS {UNIQUE} IMMEDIATE; SET CONSTRAINTS {UNIQUE} DEFERRED"
    )))
    .execute(&mut *tx)
    .await;
    match checked {
        Ok(_) => Ok(()),
        Err(source) => Err(collision(source)),
    }
}

/// The answer to `source`, a failed check of the record subjects' deferred
/// uniqueness or a failed `COMMIT` of a transaction that wrote them: the `409`
/// of a reference another `PERSON` declares, else the storage error itself.
pub(crate) fn collision(source: sqlx::Error) -> ServiceError {
    let collides = source
        .as_database_error()
        .is_some_and(|db| db.is_unique_violation() && db.constraint() == Some(UNIQUE));
    if !collides {
        return ServiceError::Storage(StorageError::Database(source));
    }
    ServiceError::Conflict(
        Refusal::new(
            Clash::State,
            format!(
                "a {TERMINOLOGY_ID_ACCESS}::{} reference this commit declares is declared by another \
                 PERSON: one EHR subject reference stands for one PERSON, so a contribution that \
                 moves it also removes it from the PERSON that held it",
                RecordSubject::RecordSubject.code()
            ),
        )
        .with_source(source),
    )
}

/// The `complete` or `incomplete` `PERSON` that declares `subject`, read on
/// `conn` from the unique constraint of `binding_record_subject`, unless a
/// received PMIR message has split it off that `PERSON`
/// ([`crate::service::demographic::identity_feed`]).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn declaring(
    conn: &mut PgConnection,
    subject: &EhrSubject,
) -> Result<Option<Uuid>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT r.holder_id FROM binding_record_subject r \
         WHERE r.namespace = $1 AND r.subject_id = $2 AND r.lifecycle_state IN ('532', '553') \
         AND NOT EXISTS (SELECT 1 FROM pmir_suppression s WHERE s.kind = 'subject' \
           AND s.holder_id = r.holder_id AND s.namespace = r.namespace \
           AND lower(s.value) = lower(r.subject_id))",
    )
    .bind(&subject.namespace)
    .bind(&subject.id)
    .fetch_optional(&mut *conn)
    .await?)
}

/// The record subjects each of `persons` declares while it is `complete` or
/// `incomplete`, read on `conn` from the primary key of
/// `binding_record_subject`, by `PERSON` and position, leaving out each one a
/// received PMIR message has split off.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn declared_by(
    conn: &mut PgConnection,
    persons: &[Uuid],
) -> Result<Vec<(Uuid, EhrSubject)>, StorageError> {
    if persons.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT r.holder_id, r.namespace, r.subject_id FROM binding_record_subject r \
         WHERE r.holder_id = ANY($1) AND r.lifecycle_state IN ('532', '553') \
         AND NOT EXISTS (SELECT 1 FROM pmir_suppression s WHERE s.kind = 'subject' \
           AND s.holder_id = r.holder_id AND s.namespace = r.namespace \
           AND lower(s.value) = lower(r.subject_id)) \
         ORDER BY r.holder_id, r.ordinal",
    )
    .bind(persons)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(person, namespace, id)| (person, EhrSubject::new(namespace, id)))
        .collect())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn element(code: &str, value: &Value) -> Value {
        json!({"_type": "ELEMENT", "archetype_node_id": "id36",
               "name": {"_type": "DV_CODED_TEXT", "value": code,
                        "defining_code": {"terminology_id": {"value": TERMINOLOGY_ID_ACCESS}, "code_string": code}},
               "value": value})
    }

    fn pseudonym(id: &str) -> Value {
        json!({"_type": "DV_IDENTIFIER", "issuer": "urn:oid:2.999.7.1", "id": id, "type": "PI"})
    }

    fn derived(kind: Kind, body: &Value) -> GrantFacts {
        let mut facts = GrantFacts::default();
        declared(kind, body, &mut facts);
        facts
    }

    #[test]
    fn a_person_declares_each_reference_in_its_details_once() {
        let body = json!({"_type": "PERSON", "details": {"items": [
            element("record_subject", &pseudonym("p-a")),
            {"items": [element("record_subject", &pseudonym("p-b"))]},
            element("record_subject", &pseudonym("p-a"))
        ]}});
        let facts = derived(Kind::Person, &body);
        assert_eq!(facts.unread, Vec::<String>::new());
        let ids: Vec<(&str, i32)> = facts
            .subjects
            .iter()
            .map(|d| (d.subject.id.as_str(), d.ordinal))
            .collect();
        assert_eq!(ids, vec![("p-a", 0), ("p-b", 1)]);
        assert_eq!(facts.subjects[0].subject.namespace, "urn:oid:2.999.7.1");
    }

    #[test]
    fn a_value_that_is_no_identifier_with_issuer_and_id_is_unread() {
        for value in [
            json!({"_type": "DV_TEXT", "value": "p-a"}),
            json!({"_type": "DV_IDENTIFIER", "id": "p-a"}),
            json!({"_type": "DV_IDENTIFIER", "issuer": "urn:oid:2.999.7.1", "id": " "}),
        ] {
            let body = json!({"_type": "PERSON", "details": {"items": [element("record_subject", &value)]}});
            let facts = derived(Kind::Person, &body);
            assert_eq!(facts.subjects, Vec::new());
            assert_eq!(facts.unread.len(), 1, "{value}");
        }
    }

    #[test]
    fn another_kind_or_another_place_holds_no_record_subject() {
        let body = json!({"details": {"items": [element("record_subject", &pseudonym("p-a"))]}});
        for kind in [Kind::Organisation, Kind::Group, Kind::Agent, Kind::Role] {
            let facts = derived(kind, &body);
            assert_eq!(facts.subjects, Vec::new());
            assert_eq!(facts.unread.len(), 1, "{kind:?}");
        }
        let person = json!({"_type": "PERSON",
            "identities": [{"details": {"items": [element("record_subject", &pseudonym("p-a"))]}}]});
        let facts = derived(Kind::Person, &person);
        assert_eq!(facts.subjects, Vec::new());
        assert_eq!(facts.unread.len(), 1);
    }

    #[test]
    fn an_element_of_another_code_is_no_record_subject() {
        let body = json!({"_type": "PERSON", "details": {"items": [element("member_of", &pseudonym("p-a"))]}});
        let facts = derived(Kind::Person, &body);
        assert_eq!(facts.subjects, Vec::new());
        assert_eq!(facts.unread, Vec::<String>::new());
    }

    fn facts_declaring(namespace: &str, id: &str) -> Facts {
        let mut facts = Facts::default();
        facts.grants.subjects.push(DeclaredSubject {
            ordinal: 0,
            subject: EhrSubject::new(namespace, id),
        });
        facts
    }

    #[test]
    fn the_own_namespace_and_an_id_the_subject_rule_refuses_are_refused() {
        let privacy = PrivacyPolicy::from(&crate::privacy::config::PrivacyConfig {
            subject_namespaces: vec!["urn:oid:2.999.7.1".to_owned()],
            ..Default::default()
        });
        let uuid = "6f1c2b9e-4d3a-4e8f-9b7c-1a2d3e4f5a6b";
        let own = facts_declaring("demographic", uuid);
        assert!(matches!(
            check(Some(&own), "demographic", &privacy),
            Err(ServiceError::Unprocessable { .. })
        ));
        let not_uuid = facts_declaring("urn:oid:2.999.7.1", "1793131381");
        assert!(matches!(
            check(Some(&not_uuid), "demographic", &privacy),
            Err(ServiceError::Unprocessable { .. })
        ));
        let pseudonym = facts_declaring("urn:oid:2.999.7.1", uuid);
        assert!(check(Some(&pseudonym), "demographic", &privacy).is_ok());
        let other = facts_declaring("benchmark", "1793131381");
        assert!(check(Some(&other), "demographic", &privacy).is_ok());
        assert!(check(None, "demographic", &privacy).is_ok());
    }
}
