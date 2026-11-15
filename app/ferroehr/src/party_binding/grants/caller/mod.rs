// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller facts: what the access evaluator reads about a caller beside
//! its grants, and the care relationships of a subject.
//!
//! A `ROLE` whose `name` is a `DV_CODED_TEXT` of a terminology the
//! jurisdiction names is a role its `performer` holds while the `ROLE` is in
//! force: the role's type is its name (RM demographic
//! `org.openehr.rm.demographic.party.adoc`, `PARTY.type()` with the invariant
//! `type = name`, and `org.openehr.rm.demographic.role.adoc`). The rules of
//! the controller's `ACCESS_POLICY` name callers by such a role
//! (`ACCESS_ACTOR_SCOPE.role`), so the demographic service is the one source
//! of who holds which role. A
//! `PARTY_RELATIONSHIP` of a `PERSON` whose `name` or `details` carry a code
//! of [`CareRelationship`] is a care relationship
//! (`org.openehr.rm.demographic.party_relationship.adoc`: `source`, `target`,
//! `time_validity`): the professional is a member (HL7 v3 `RoleClass#MBR`) of a care team, and the
//! patient is `under_care_of` a team or a unit, with the admission as its
//! `time_validity`. The care delivery environment, such as a hospital's ADT
//! system, writes them under a [`Operation::WriteCareRelationship`] grant
//! (BASE Architecture Overview, `master07-security.adoc` §The Problem of
//! Roles: the evaluation of "care deliverer" into identities "must be done in
//! each care delivery environment, not in the EHR").
//!
//! A `PARTY_RELATIONSHIP` of an `ORGANISATION` coded [`CareRelationship::MemberOf`]
//! is its membership of a `GROUP`, such as the providers of a region (RM
//! demographic `org.openehr.rm.demographic.group.adoc`), written under the
//! same grant. A client acting for that `ORGANISATION` acts for the `GROUP`
//! too ([`CallerFacts::provider_groups`]): one level of membership, our own
//! design (#5575).
//!
//! [`store`] writes the facts in each commit's transaction and stamps the
//! parties whose facts changed, and [`crate::party_binding::grants::live`]
//! holds them on each replica beside the grants, so a request reads them with
//! no transaction of its own. Nothing is read by archetype: a role is read by
//! its coded name, every other fact by its code. No openEHR spec gives a role
//! or a relationship an access meaning: our own design (#5621).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the stored canonical party body \
              the caller facts are derived from"
)]

pub mod store;
pub mod subject;

use serde_json::Value;
use uuid::Uuid;

use crate::error::{ServiceError, Violation};
use crate::party_binding::BindingMapping;
use crate::party_binding::derive::{Facts, container};
use crate::party_binding::grants::capabilities::Capabilities;
use crate::party_binding::grants::derive::{Bound, GrantFacts};
use crate::party_binding::grants::operation::Operation;
use crate::party_binding::grants::store::Dates;
use crate::party_binding::policy_mark::{TERMINOLOGY, carries};
use crate::storage::error::StorageError;
use crate::versioning::Kind;
use openehr_rm::draft::access_control::terminology::{
    CareRelationship, ForeignCode, ForeignKind, Term,
};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

/// What a caller fact is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FactKind {
    /// The performer holds the role the `ROLE`'s coded name names.
    Role,
    /// The professional is a member of the care team.
    MemberOf,
    /// The patient is in the care of the team or unit.
    UnderCareOf,
}

impl FactKind {
    /// Every kind, in contract order.
    const ALL: [Self; 3] = [Self::Role, Self::MemberOf, Self::UnderCareOf];

    /// The care relationship term the kind is read by; `None` for a role,
    /// which the `ROLE`'s coded name names in the jurisdiction's terminology.
    pub(crate) fn care(self) -> Option<CareRelationship> {
        match self {
            Self::Role => None,
            Self::MemberOf => Some(CareRelationship::MemberOf),
            Self::UnderCareOf => Some(CareRelationship::UnderCareOf),
        }
    }

    /// The kind as the side table stores it: `role` for a role, and the code
    /// a care relationship is read by.
    pub(crate) fn code(self) -> &'static str {
        self.care().map_or("role", |care| care.code())
    }

    /// The kind stored as `code`, `None` for another code.
    pub(crate) fn of(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }
}

impl From<CareRelationship> for FactKind {
    fn from(term: CareRelationship) -> Self {
        match term {
            CareRelationship::MemberOf => Self::MemberOf,
            CareRelationship::UnderCareOf => Self::UnderCareOf,
        }
    }
}

/// What one fact names: the `CODE_PHRASE` of a role, or the container a care
/// relationship targets.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FactValue {
    /// A role code of the terminology the jurisdiction names.
    Code(ForeignCode),
    /// The care team `GROUP` or the organisational unit.
    Team(Uuid),
}

/// One caller fact a body holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CallerFact {
    /// The relationship's position; `0` for a role.
    pub(crate) ordinal: i32,
    /// What the fact is.
    pub(crate) kind: FactKind,
    /// What it names.
    pub(crate) value: FactValue,
    /// The relationship's `time_validity`; `None` for a role, which holds
    /// while its `ROLE` does.
    pub(crate) validity: Option<Value>,
    /// Always [`Bound::Open`]: no fact has a credentials bound.
    pub(crate) not_before: Bound,
    /// Always [`Bound::Open`].
    pub(crate) not_after: Bound,
}

/// Pushes onto `facts` the role `body`, a `ROLE`, names: its `name` when that
/// is a `DV_CODED_TEXT` of a terminology the jurisdiction names. A `ROLE`
/// named in text, or by a code of [`crate::party_binding::policy_mark::TERMINOLOGY`], which holds no role,
/// names none.
pub(crate) fn role(body: &Value, facts: &mut GrantFacts) {
    let Some(name) = body
        .get("name")
        .filter(|name| name.get("_type").and_then(Value::as_str) == Some("DV_CODED_TEXT"))
    else {
        return;
    };
    let coded: DvCodedText = match openehr_its::json::from_canonical_value(name) {
        Ok(coded) => coded,
        Err(error) => {
            facts.unread.push(format!(
                "ROLE.name: the DV_CODED_TEXT does not read: {error}"
            ));
            return;
        }
    };
    // NOTE: our own design (#5621), a rule names a role only in a jurisdiction's terminology,
    // so a ROLE named by an operation code is legitimately no role a rule can match.
    if let Ok(code) = ForeignCode::read(ForeignKind::Role, &coded) {
        facts.facts.push(CallerFact {
            ordinal: 0,
            kind: FactKind::Role,
            value: FactValue::Code(code),
            validity: None,
            not_before: Bound::Open,
            not_after: Bound::Open,
        });
    }
}

/// Pushes onto `facts` the care relationships `body`, a party of `kind`,
/// holds, and why one cannot be read: a care relationship is a relationship
/// of a `PERSON`, its source, to a party of this store, its target, and a
/// membership (`MBR`) of an `ORGANISATION` is its membership of a `GROUP`, such as
/// the providers of a region.
pub(crate) fn relationships(kind: Kind, body: &Value, facts: &mut GrantFacts) {
    if kind == Kind::PartyRelationship {
        if let Some(term) = care_kind(body) {
            facts.unread.push(format!(
                "a {}::{} relationship is held in its source PERSON's \
                 relationships, never as a PARTY_RELATIONSHIP of its own",
                term.terminology(),
                term.code()
            ));
        }
        return;
    }
    let held = body.get("relationships").and_then(Value::as_array);
    for (ordinal, relationship) in held.into_iter().flatten().enumerate() {
        let Some(code) = care_kind(relationship) else {
            continue;
        };
        let care = FactKind::from(code);
        let target = relationship
            .pointer("/target/id/value")
            .and_then(Value::as_str)
            .and_then(container);
        let refusal = match (kind, care, target) {
            (Kind::Person, _, Some(team))
            | (Kind::Organisation, FactKind::MemberOf, Some(team)) => {
                facts.facts.push(CallerFact {
                    ordinal: i32::try_from(ordinal).unwrap_or(i32::MAX),
                    kind: care,
                    value: FactValue::Team(team),
                    validity: relationship.get("time_validity").cloned(),
                    not_before: Bound::Open,
                    not_after: Bound::Open,
                });
                continue;
            }
            (Kind::Person, _, None) | (Kind::Organisation, FactKind::MemberOf, None) => {
                "targets no party of this store by its container"
            }
            (Kind::Organisation, _, _) => {
                "is a relationship of a PERSON, its source; an ORGANISATION holds only a membership"
            }
            _ => "is a relationship of a PERSON, or a membership of an ORGANISATION, its source",
        };
        facts.unread.push(format!(
            "relationships[{ordinal}]: a {}::{} relationship {refusal}",
            code.terminology(),
            code.code()
        ));
    }
}

/// The care relationship code `relationship` carries in its `name` or its
/// `details`, the first of [`CareRelationship`] in contract order.
fn care_kind(relationship: &Value) -> Option<CareRelationship> {
    CareRelationship::ALL
        .iter()
        .copied()
        .find(|term| carries(relationship, term.terminology(), term.code()))
}

/// Whether the stored `PERSON` bodies `before` and `after`, two versions of
/// one party, differ only in their care relationships: each with its `uid`
/// and every relationship carrying a [`CareRelationship`] code taken out is
/// the same tree.
///
/// The party protection leaves such a version to [`check_care_writer`], so a
/// caller holding [`Operation::WriteCareRelationship`] rosters a provisioned
/// professional and changes nothing else (#5617). Our own design.
///
/// # Errors
/// [`StorageError::BodyDecode`] when a stored body is no JSON.
pub(crate) fn only_care_differs(before: &str, after: &str) -> Result<bool, StorageError> {
    let without_care = |body: &str| -> Result<Value, StorageError> {
        let mut value: Value = serde_json::from_str(body).map_err(StorageError::BodyDecode)?;
        if let Some(object) = value.as_object_mut() {
            object.remove("uid");
            if let Some(Value::Array(relationships)) = object.get_mut("relationships") {
                relationships.retain(|relationship| care_kind(relationship).is_none());
                if relationships.is_empty() {
                    object.remove("relationships");
                }
            }
        }
        Ok(value)
    };
    Ok(without_care(before)? == without_care(after)?)
}

/// Refuses the committed body `facts` when a caller fact it carries cannot be
/// read: a `ROLE` name that is a `DV_CODED_TEXT` that does not read, and a
/// care relationship that is not a `PERSON`'s or names no target of this
/// store.
///
/// # Errors
/// [`ServiceError::Unprocessable`] (`422`) naming the first such fact.
pub(crate) fn check_content(facts: Option<&Facts>) -> Result<(), ServiceError> {
    match facts.and_then(|facts| facts.grants.unread.first()) {
        Some(unread) => Err(ServiceError::content_invalid(Violation::new(format!(
            "a caller fact cannot be read: {unread}"
        )))),
        None => Ok(()),
    }
}

/// Refuses the commit `committed` of a `PERSON` or an `ORGANISATION` when
/// its version changes what the care relationships or memberships say, or
/// the record subjects it declares, and its caller may not
/// ([`check_care_writer`]).
///
/// The care relationships and record subjects of `facts`, what the committed
/// body holds, are compared with those of the last earlier version that
/// holds a body: a relationship by kind, target and `time_validity`, a
/// subject by namespace and id, never by position (#5640). A logical
/// deletion holds no body and changes none: its earlier versions keep them
/// (RM common `master06-change_control_package.adoc` §Logical Deletion). A
/// lifecycle state is no relationship content, so a deactivation that keeps
/// them needs no grant (#5628). Our own design.
///
/// # Errors
/// The `403` of [`check_care_writer`], and a storage failure.
pub(crate) async fn check_care(
    tx: &mut sqlx::PgConnection,
    mapping: &BindingMapping,
    committed: &crate::versioning::change::Committed,
    facts: Option<&Facts>,
) -> Result<(), ServiceError> {
    if !matches!(committed.kind, Kind::Person | Kind::Organisation) {
        return Ok(());
    }
    let Some(after) = facts else {
        return Ok(());
    };
    let before: Option<String> = sqlx::query_scalar(
        "SELECT v.body FROM version v WHERE v.vo_id = $1 AND v.sys_version < $2 \
         AND v.body IS NOT NULL ORDER BY v.sys_version DESC LIMIT 1",
    )
    .bind(committed.vo_id)
    .bind(committed.sys_version)
    .fetch_optional(&mut *tx)
    .await
    .map_err(StorageError::from)?;
    let before = match before {
        Some(body) => {
            let body: Value = serde_json::from_str(&body).map_err(StorageError::BodyDecode)?;
            GrantFacts::of(committed.kind, &body)
        }
        None => GrantFacts::default(),
    };
    if same_care(&before.facts, &after.grants.facts)
        && same_subjects(&before.subjects, &after.grants.subjects)
    {
        return Ok(());
    }
    check_care_writer(mapping, committed.vo_id.0)
}

/// Whether the care relationships among `before` and `after` say the same:
/// each of one matches one of the other by kind, target and
/// `time_validity`, whatever their order.
fn same_care(before: &[CallerFact], after: &[CallerFact]) -> bool {
    let care = |fact: &&CallerFact| matches!(fact.value, FactValue::Team(_));
    let mut unmatched: Vec<&CallerFact> = after.iter().filter(care).collect();
    before.iter().filter(care).all(|fact| {
        let found = unmatched.iter().position(|other| {
            other.kind == fact.kind && other.value == fact.value && other.validity == fact.validity
        });
        found.map(|at| unmatched.swap_remove(at)).is_some()
    }) && unmatched.is_empty()
}

/// Whether `before` and `after` declare the same record subjects, by
/// namespace and id, whatever their order.
fn same_subjects(before: &[subject::DeclaredSubject], after: &[subject::DeclaredSubject]) -> bool {
    let sorted = |held: &[subject::DeclaredSubject]| {
        let mut subjects: Vec<_> = held
            .iter()
            .map(|declared| declared.subject.clone())
            .collect();
        subjects.sort();
        subjects
    };
    sorted(before) == sorted(after)
}

/// Whether `relationship` carries a code of [`CareRelationship`] in its
/// `name` or its `details`: a care relationship, which SCIM never changes.
pub(crate) fn is_care(relationship: &Value) -> bool {
    care_kind(relationship).is_some()
}

/// Refuses a commit that changed the care relationships, memberships or
/// record subjects of the party `party` when its caller holds no
/// [`Operation::WriteCareRelationship`] grant and is no administrator of
/// `mapping`.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming the grant.
fn check_care_writer(mapping: &BindingMapping, party: Uuid) -> Result<(), ServiceError> {
    if Capabilities::current().holds(Operation::WriteCareRelationship)
        || crate::party_binding::protect::administrator(mapping)
    {
        return Ok(());
    }
    Err(ServiceError::not_permitted(format!(
        "this version changes the care relationships of party {party} ({}::{} or {}::{}) or \
         the record subjects it declares ({TERMINOLOGY}::{}): only a caller holding the {} \
         grant, such as the care delivery environment's ADT system, or an administrator \
         writes them",
        CareRelationship::MemberOf.terminology(),
        CareRelationship::MemberOf.code(),
        CareRelationship::UnderCareOf.terminology(),
        CareRelationship::UnderCareOf.code(),
        openehr_rm::draft::access_control::terminology::RecordSubject::RecordSubject.code(),
        Operation::WriteCareRelationship
    )))
}

/// One care relationship: the team or unit it targets and the dates its
/// `time_validity` holds.
///
/// The evaluator reads a caller's from [`CallerFacts::care_teams`] and a
/// subject's from
/// [`crate::party_binding::grants::live::LiveGrants::under_care_of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CareTeam {
    team: Uuid,
    dates: Dates,
}

impl CareTeam {
    /// The relationship to `team` over `dates`.
    pub(crate) fn new(team: Uuid, dates: Dates) -> Self {
        Self { team, dates }
    }

    /// The relationship to `team` from the date `from` until the first date
    /// it no longer holds, `until`, each `None` when unbounded.
    #[must_use]
    pub fn spanning(
        team: Uuid,
        from: Option<jiff::civil::Date>,
        until: Option<jiff::civil::Date>,
    ) -> Self {
        Self::new(
            team,
            Dates {
                lower: from,
                upper: until,
            },
        )
    }

    /// The container of the care team `GROUP` or the organisational unit.
    #[must_use]
    pub fn team(&self) -> Uuid {
        self.team
    }

    /// The first date it holds; `None` when unbounded.
    #[must_use]
    pub fn from(&self) -> Option<jiff::civil::Date> {
        self.dates.lower
    }

    /// The first date it no longer holds, such as the day after a
    /// discharge; `None` when unbounded.
    #[must_use]
    pub fn until(&self) -> Option<jiff::civil::Date> {
        self.dates.upper
    }

    /// Whether it holds on `day`.
    #[must_use]
    pub fn holds_on(&self, day: jiff::civil::Date) -> bool {
        self.dates.holds(day)
    }

    /// Whether it admits access on `day`: it has begun, and it holds or ended
    /// less than `grace` before `day` (BASE Architecture Overview
    /// `master07-security.adoc` §Security Policy, "Time-limitation of
    /// access": the episode of care "plus some further time").
    #[must_use]
    pub fn live_on(&self, day: jiff::civil::Date, grace: jiff::Span) -> bool {
        let begun = self.dates.lower.is_none_or(|lower| lower <= day);
        // NOTE: our own design, a grace that does not add to the end date admits nothing after it.
        let open = self
            .dates
            .upper
            .is_none_or(|upper| day < upper || upper.checked_add(grace).is_ok_and(|end| day < end));
        begun && open
    }
}

/// What the access evaluator reads about one caller beside its grants.
///
/// They are the roles of the `ROLE`s in force the caller performs, the care
/// teams it is a member of on the request's date, and the `GROUP`s the
/// provider a client acts for is a member of on that date.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallerFacts {
    roles: Vec<ForeignCode>,
    care_teams: Vec<CareTeam>,
    provider_groups: Vec<Uuid>,
}

impl CallerFacts {
    /// The facts holding `roles` and `care_teams`, each sorted and distinct.
    #[must_use]
    pub fn new(mut roles: Vec<ForeignCode>, mut care_teams: Vec<CareTeam>) -> Self {
        roles.sort();
        roles.dedup();
        care_teams.sort();
        care_teams.dedup();
        Self {
            roles,
            care_teams,
            provider_groups: Vec::new(),
        }
    }

    /// These facts with `groups`, the `GROUP`s the client's provider is a
    /// member of, sorted and distinct.
    #[must_use]
    pub fn with_provider_groups(mut self, mut groups: Vec<Uuid>) -> Self {
        groups.sort_unstable();
        groups.dedup();
        self.provider_groups = groups;
        self
    }

    /// The roles of the `ROLE`s in force the caller performs.
    #[must_use]
    pub fn roles(&self) -> &[ForeignCode] {
        &self.roles
    }

    /// Whether the caller performs a `ROLE` in force named `role`, compared
    /// by terminology id and code (`ACCESS_ACTOR_SCOPE.role`).
    #[must_use]
    pub fn performs(&self, role: &DvCodedText) -> bool {
        ForeignCode::read(ForeignKind::Role, role)
            .is_ok_and(|code| self.roles.binary_search(&code).is_ok())
    }

    /// The care teams the caller is a member of on the request's date.
    #[must_use]
    pub fn care_teams(&self) -> &[CareTeam] {
        &self.care_teams
    }

    /// The containers of the `GROUP`s the provider a client acts for is a
    /// member of on the request's date, by a membership (`MBR`) relationship of the
    /// provider `ORGANISATION`.
    #[must_use]
    pub fn provider_groups(&self) -> &[Uuid] {
        &self.provider_groups
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn coded(terminology: &str, code: &str) -> Value {
        json!({"_type": "DV_CODED_TEXT", "value": code,
               "defining_code": {"_type": "CODE_PHRASE",
                                 "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
                                 "code_string": code}})
    }

    fn element(name: &Value, value: &Value) -> Value {
        json!({"_type": "ELEMENT", "archetype_node_id": "at1", "name": name, "value": value})
    }

    fn code(terminology: &str, code: &str) -> ForeignCode {
        ForeignCode {
            terminology: terminology.to_owned(),
            code: code.to_owned(),
        }
    }

    const TEAM: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c01308";

    /// A professional's membership of a care team: HL7 v3 `RoleClass#MBR`.
    const MEMBER: (&str, &str) = ("v3-RoleClass", "MBR");
    /// A patient under the care of a team or unit.
    const UNDER_CARE: (&str, &str) = (TERMINOLOGY, "under_care_of");
    /// A representation.
    const REPRESENTS: (&str, &str) = (TERMINOLOGY, "represents");

    fn relationship((terminology, code): (&str, &str), target: &str) -> Value {
        json!({"_type": "PARTY_RELATIONSHIP",
               "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.ward.v4",
               "name": {"_type": "DV_TEXT", "value": "ward"},
               "details": {"_type": "ITEM_TREE", "archetype_node_id": "at2",
                           "items": [element(&json!({"_type": "DV_TEXT", "value": "kind"}),
                                             &coded(terminology, code))]},
               "time_validity": {"_type": "DV_INTERVAL",
                                 "lower": {"_type": "DV_DATE", "value": "2026-11-03"},
                                 "lower_unbounded": false, "upper_unbounded": true},
               "target": {"id": {"_type": "HIER_OBJECT_ID", "value": target}}})
    }

    #[test]
    fn a_role_is_read_from_the_coded_name_of_a_role() {
        let role = |name: Value| {
            json!({"_type": "ROLE",
                   "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-ROLE.nurse.v2",
                   "name": name,
                   "capabilities": [{"_type": "CAPABILITY", "credentials": {"items": [
                       element(&json!({"_type": "DV_TEXT", "value": "operation"}),
                               &coded(TERMINOLOGY, "access_demographic"))]}}]})
        };
        let facts = GrantFacts::of(Kind::Role, &role(coded("urn:example:roles", "NURSE")));
        let read: Vec<(i32, FactKind, FactValue)> = facts
            .facts
            .iter()
            .map(|fact| (fact.ordinal, fact.kind, fact.value.clone()))
            .collect();
        assert_eq!(
            read,
            vec![(
                0,
                FactKind::Role,
                FactValue::Code(code("urn:example:roles", "NURSE"))
            )]
        );
        assert!(
            facts.facts[0].validity.is_none(),
            "a role holds while its ROLE does"
        );
        assert_eq!(facts.unread, Vec::<String>::new());
        let text = GrantFacts::of(
            Kind::Role,
            &role(json!({"_type": "DV_TEXT", "value": "Nurse"})),
        );
        assert_eq!(text.facts, Vec::new(), "a ROLE named in text names no role");
        let operation = GrantFacts::of(Kind::Role, &role(coded(TERMINOLOGY, "NURSE")));
        assert_eq!(
            operation.facts,
            Vec::new(),
            "an operation code names no role"
        );
        assert_eq!(operation.unread, Vec::<String>::new());
    }

    #[test]
    fn a_persons_care_relationships_are_read_by_their_code_in_any_archetype() {
        let body = json!({"_type": "PERSON", "relationships": [
            relationship(MEMBER, TEAM),
            relationship(REPRESENTS, TEAM),
            relationship(UNDER_CARE, TEAM),
            relationship(UNDER_CARE, "1.2.840.113619")
        ]});
        let facts = GrantFacts::of(Kind::Person, &body);
        let team = Uuid::parse_str(TEAM).expect("a uuid");
        let read: Vec<(i32, FactKind, FactValue)> = facts
            .facts
            .iter()
            .map(|fact| (fact.ordinal, fact.kind, fact.value.clone()))
            .collect();
        assert_eq!(
            read,
            vec![
                (0, FactKind::MemberOf, FactValue::Team(team)),
                (2, FactKind::UnderCareOf, FactValue::Team(team)),
            ]
        );
        assert!(facts.facts[0].validity.is_some());
        assert_eq!(facts.unread.len(), 1, "{:?}", facts.unread);
        assert!(facts.unread[0].starts_with("relationships[3]"));
    }

    #[test]
    fn a_membership_is_read_by_the_hl7_code_alone() {
        let body = json!({"_type": "PERSON", "relationships": [
            relationship((TERMINOLOGY, "member_of"), TEAM),
            relationship(("v3-RoleCode", "MBR"), TEAM)
        ]});
        let facts = GrantFacts::of(Kind::Person, &body);
        assert_eq!(facts.facts, Vec::new());
        assert_eq!(facts.unread, Vec::<String>::new());
    }

    #[test]
    fn a_care_relationship_is_a_persons_and_never_one_of_its_own() {
        let body = json!({"_type": "GROUP", "relationships": [relationship(MEMBER, TEAM)]});
        let facts = GrantFacts::of(Kind::Group, &body);
        assert_eq!(facts.facts, Vec::new());
        assert_eq!(facts.unread.len(), 1);
        let standalone = GrantFacts::of(Kind::PartyRelationship, &relationship(UNDER_CARE, TEAM));
        assert_eq!(standalone.unread.len(), 1);
        assert_eq!(
            GrantFacts::of(Kind::PartyRelationship, &relationship(REPRESENTS, TEAM)).unread,
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_organisations_member_of_is_its_membership_of_a_group() {
        let body = json!({"_type": "ORGANISATION", "relationships": [
            relationship(MEMBER, TEAM),
            relationship(UNDER_CARE, TEAM),
            relationship(MEMBER, "1.2.840.113619")
        ]});
        let facts = GrantFacts::of(Kind::Organisation, &body);
        let group = Uuid::parse_str(TEAM).expect("a uuid");
        let read: Vec<(i32, FactKind, FactValue)> = facts
            .facts
            .iter()
            .map(|fact| (fact.ordinal, fact.kind, fact.value.clone()))
            .collect();
        assert_eq!(read, vec![(0, FactKind::MemberOf, FactValue::Team(group))]);
        assert_eq!(facts.unread.len(), 2, "{:?}", facts.unread);
        assert!(
            facts.unread[0].starts_with("relationships[1]")
                && facts.unread[0].contains("holds only a membership"),
            "an ORGANISATION is never under care: {:?}",
            facts.unread
        );
        assert!(facts.unread[1].starts_with("relationships[2]"));
    }

    #[test]
    fn care_compares_what_relationships_say_never_their_order_or_other_facts() {
        let facts = |relationships: Vec<Value>| {
            GrantFacts::of(
                Kind::Person,
                &json!({"_type": "PERSON", "relationships": relationships}),
            )
            .facts
        };
        let other = "0190a1bc-3a1e-72e4-b7c5-f78a24c01309";
        let both = facts(vec![
            relationship(MEMBER, TEAM),
            relationship(UNDER_CARE, other),
        ]);
        let swapped = facts(vec![
            relationship(REPRESENTS, TEAM),
            relationship(UNDER_CARE, other),
            relationship(MEMBER, TEAM),
        ]);
        assert!(same_care(&both, &swapped), "order and other relationships");
        assert!(!same_care(&both, &both[..1]), "a relationship removed");
        assert!(!same_care(&both[..1], &both), "a relationship added");
        let mut ended = relationship(MEMBER, TEAM);
        ended["time_validity"]["upper"] = json!({"_type": "DV_DATE", "value": "2026-11-14"});
        ended["time_validity"]["upper_unbounded"] = json!(false);
        let ended = facts(vec![ended, relationship(UNDER_CARE, other)]);
        assert!(!same_care(&both, &ended), "a relationship ended");
        let twice = facts(vec![relationship(MEMBER, TEAM), relationship(MEMBER, TEAM)]);
        assert!(!same_care(&twice, &both[..1]), "a repeat removed");
    }

    #[test]
    fn record_subjects_compare_by_reference_never_by_position() {
        let declared = |ordinal: i32, id: &str| subject::DeclaredSubject {
            ordinal,
            subject: crate::storage::access_projection::EhrSubject::new("pixm", id),
        };
        let both = [declared(0, "p-1"), declared(1, "p-2")];
        let swapped = [declared(0, "p-2"), declared(1, "p-1")];
        assert!(same_subjects(&both, &swapped));
        assert!(!same_subjects(&both, &both[..1]));
        assert!(!same_subjects(
            &both,
            &[declared(0, "p-1"), declared(1, "p-3")]
        ));
    }

    #[test]
    fn provider_groups_are_sorted_and_distinct() {
        let (a, b) = (Uuid::from_u128(2), Uuid::from_u128(1));
        let facts = CallerFacts::default().with_provider_groups(vec![a, b, a]);
        assert_eq!(facts.provider_groups(), &[b, a]);
    }

    #[test]
    fn every_kind_reads_back_from_its_code() {
        for kind in FactKind::ALL {
            assert_eq!(FactKind::of(kind.code()), Some(kind));
        }
        assert_eq!(FactKind::of("represents"), None);
    }

    #[test]
    fn caller_facts_answer_by_terminology_and_code() {
        let nurse = code("urn:example:roles", "NURSE");
        let facts = CallerFacts::new(vec![nurse.clone(), nurse], Vec::new());
        assert_eq!(facts.roles().len(), 1);
        let named = |terminology: &str, value: &str| -> DvCodedText {
            openehr_its::json::from_canonical_value(&coded(terminology, value)).expect("a code")
        };
        assert!(facts.performs(&named("urn:example:roles", "NURSE")));
        assert!(!facts.performs(&named("local", "NURSE")));
        assert!(!facts.performs(&named("urn:example:roles", "nurse")));
    }

    #[test]
    fn a_care_team_holds_within_its_dates() {
        let day = |text: &str| text.parse::<jiff::civil::Date>().expect("a date");
        let team = CareTeam::new(
            Uuid::nil(),
            Dates {
                lower: Some(day("2026-11-03")),
                upper: Some(day("2026-11-11")),
            },
        );
        assert!(team.holds_on(day("2026-11-03")) && team.holds_on(day("2026-11-10")));
        assert!(!team.holds_on(day("2026-11-11")) && !team.holds_on(day("2026-11-02")));
        assert_eq!(team.until(), Some(day("2026-11-11")));
        assert_eq!(team.from(), Some(day("2026-11-03")));
    }
}
