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
//! `time_validity`): the professional is a `member_of` a care team, and the
//! patient is `under_care_of` a team or a unit, with the admission as its
//! `time_validity`. The care delivery environment, such as a hospital's ADT
//! system, writes them under a `write_care_relationship` grant
//! (BASE Architecture Overview, `master07-security.adoc` §The Problem of
//! Roles: the evaluation of "care deliverer" into identities "must be done in
//! each care delivery environment, not in the EHR").
//!
//! A `PARTY_RELATIONSHIP` of an `ORGANISATION` coded [`CareRelationship::MemberOf`]
//! is its membership of a `GROUP`, such as the providers of a region (RM
//! demographic `org.openehr.rm.demographic.group.adoc`), written under the
//! same grant. A client acting for that `ORGANISATION` acts for the `GROUP`
//! too ([`facts::CallerFacts::provider_groups`]): one level of membership, our own
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

pub mod attestation;
pub(crate) mod bounds;
pub mod care_of;
pub mod care_ref;
pub mod facts;
pub(crate) mod kin;
pub mod patient;
pub mod store;
pub mod subject;
pub(crate) mod writer;

use serde_json::Value;

use crate::versioning::tree::Tree;
use uuid::Uuid;

use crate::error::{ServiceError, Violation};
use crate::party_binding::derive::{Facts, container};
use crate::party_binding::grants::derive::{Bound, GrantFacts};
use crate::party_binding::grants::store::Dates;
use crate::party_binding::policy_mark::carries;
use crate::storage::error::StorageError;
use crate::versioning::Kind;
use openehr_rm::draft::access_control::terminology::kinship::Kinship;
use openehr_rm::draft::access_control::terminology::{
    CareRelationship, ForeignCode, ForeignKind, Term,
};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

/// What a caller fact is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FactKind {
    /// The performer holds the role the `ROLE`'s coded name names.
    Role,
    /// The `ORGANISATION` is of the type its coded name names.
    OrganisationType,
    /// The professional is a member of the care team.
    MemberOf,
    /// The patient is in the care of the team or unit.
    UnderCareOf,
    /// The person is kin to another person.
    RelatedTo,
}

impl FactKind {
    /// Every kind, in contract order.
    const ALL: [Self; 5] = [
        Self::Role,
        Self::OrganisationType,
        Self::MemberOf,
        Self::UnderCareOf,
        Self::RelatedTo,
    ];

    /// The care relationship term the kind is read by; `None` for a role or
    /// an organisation type, which a party's coded name names in the
    /// jurisdiction's terminology.
    pub(crate) fn care(self) -> Option<CareRelationship> {
        match self {
            Self::Role | Self::OrganisationType | Self::RelatedTo => None,
            Self::MemberOf => Some(CareRelationship::MemberOf),
            Self::UnderCareOf => Some(CareRelationship::UnderCareOf),
        }
    }

    /// The kind as the side table stores it: `role` for a role,
    /// `organisation_type` for an organisation type, and the code a care
    /// relationship is read by.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Role => "role",
            Self::OrganisationType => "organisation_type",
            Self::MemberOf | Self::UnderCareOf => self.care().map_or("", |care| care.code()),
            Self::RelatedTo => Kinship::RelatedTo.code(),
        }
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

/// What one fact names: the `CODE_PHRASE` of a role, the container a
/// membership targets, or what a care relationship states.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FactValue {
    /// A role code of the terminology the jurisdiction names.
    Code(ForeignCode),
    /// The care team `GROUP` or the organisational unit of a membership.
    Team(Uuid),
    /// The carer, its role and the provider of a care relationship.
    Care(care_of::CareFact),
    /// The other person and the kinship of a `related_to` relationship.
    Kin(kin::KinFact),
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
    /// The party the fact is about when it is not the party its head names:
    /// each end of a `related_to` relationship; `None` for every other fact.
    pub(crate) about: Option<Uuid>,
}

/// Pushes onto `facts` the fact of `kind` the coded `name` of `body` names.
///
/// For a `ROLE` it is the role its performer holds, for an `ORGANISATION` its
/// own type, when that `name` is a `DV_CODED_TEXT` of a terminology the
/// jurisdiction names (RM demographic `PARTY.type()`, invariant
/// `type = name`). A party named in text, or by a code of
/// [`crate::party_binding::policy_mark::TERMINOLOGY`], names none, and a care
/// relationship kind names nothing here.
pub(crate) fn coded_name<'a, T: Tree<'a>>(body: T, kind: FactKind, facts: &mut GrantFacts) {
    let (class, foreign) = match kind {
        FactKind::Role => ("ROLE", ForeignKind::Role),
        FactKind::OrganisationType => ("ORGANISATION", ForeignKind::OrganisationType),
        FactKind::MemberOf | FactKind::UnderCareOf | FactKind::RelatedTo => return,
    };
    let Some(name) = body
        .get("name")
        .filter(|name| name.get_text("_type").as_deref() == Some("DV_CODED_TEXT"))
    else {
        return;
    };
    let coded: DvCodedText = match openehr_its::json::from_canonical_value(&name.to_value()) {
        Ok(coded) => coded,
        Err(error) => {
            facts.unread.push(format!(
                "{class}.name: the DV_CODED_TEXT does not read: {error}"
            ));
            return;
        }
    };
    // NOTE: our own design (#5621, #5691), a rule names a role or an organisation type only in
    // a jurisdiction's terminology, so a name coded in the access terminology names none.
    if let Ok(code) = ForeignCode::read(foreign, &coded) {
        facts.facts.push(CallerFact {
            ordinal: 0,
            kind,
            value: FactValue::Code(code),
            validity: None,
            not_before: Bound::Open,
            not_after: Bound::Open,
            about: None,
        });
    }
}

/// Pushes onto `facts` the care relationship or the memberships `body`, of
/// `kind`, holds, and why one cannot be read: a care relationship is a
/// `PARTY_RELATIONSHIP` of its own ([`care_of`]), a membership (`member_of`) is
/// held in the relationships of its member, a `PERSON` or an
/// `ORGANISATION`, such as a provider's membership of the providers of a
/// region, or a `GROUP`, whose membership names the `ORGANISATION` it
/// belongs to and so the provider that writes its memberships.
pub(crate) fn relationships<'a, T: Tree<'a>>(kind: Kind, body: T, facts: &mut GrantFacts) {
    if kind == Kind::PartyRelationship {
        if kin::is_kinship(body) {
            kin::read(body, facts);
            return;
        }
        match care_kind(body) {
            Some(CareRelationship::UnderCareOf) => care_of::read(body, facts),
            Some(term) => facts.unread.push(format!(
                "a {}::{} relationship is held in its source's relationships, never as a \
                 PARTY_RELATIONSHIP of its own",
                term.terminology(),
                term.code()
            )),
            None => {}
        }
        return;
    }
    let held = body.get("relationships");
    for (ordinal, relationship) in held.into_iter().flat_map(Tree::items).enumerate() {
        if kin::is_kinship(relationship) {
            facts.unread.push(kin::held_inline(ordinal));
            continue;
        }
        let Some(code) = care_kind(relationship) else {
            continue;
        };
        if code == CareRelationship::UnderCareOf {
            facts.unread.push(care_of::held_inline(ordinal));
            continue;
        }
        let target = relationship
            .pointer_text("/target/id/value")
            .and_then(|id| container(&id));
        let organisation = relationship.pointer_text("/target/type").as_deref()
            == Some(care_of::CarerKind::Organisation.rm_type());
        let refusal = match (kind, target) {
            (Kind::Group, Some(_)) if !organisation => {
                "of a GROUP targets the ORGANISATION the GROUP belongs to, by a PARTY_REF of \
                 type ORGANISATION"
            }
            (Kind::Person | Kind::Organisation | Kind::Group, Some(team)) => {
                facts.facts.push(CallerFact {
                    ordinal: i32::try_from(ordinal).unwrap_or(i32::MAX),
                    kind: FactKind::MemberOf,
                    value: FactValue::Team(team),
                    validity: relationship.get("time_validity").map(Tree::to_value),
                    not_before: Bound::Open,
                    not_after: Bound::Open,
                    about: None,
                });
                continue;
            }
            (Kind::Person | Kind::Organisation | Kind::Group, None) => {
                "targets no party of this store by its container"
            }
            _ => "is a membership of a PERSON, an ORGANISATION or a GROUP, its source",
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
fn care_kind<'a, T: Tree<'a>>(relationship: T) -> Option<CareRelationship> {
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
/// The party protection leaves such a version to [`writer::check_care`], so a
/// caller holding a `write_care_relationship` grant rosters a provisioned
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

/// Whether `relationship` carries a code of [`CareRelationship`] in its
/// `name` or its `details`: a care relationship, which SCIM never changes.
pub(crate) fn is_care<'a, T: Tree<'a>>(relationship: T) -> bool {
    care_kind(relationship).is_some()
}

/// One membership of a care team or unit: the team or unit it targets and
/// the dates its `time_validity` holds.
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
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::writer::{care_changes, same_subjects};
    use super::*;
    use crate::party_binding::policy_mark::TERMINOLOGY;

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

    /// A professional's membership of a care team.
    const MEMBER: (&str, &str) = (TERMINOLOGY, "member_of");
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
    fn an_organisation_type_is_read_from_the_coded_name_of_an_organisation() {
        let organisation = |name: Value| {
            json!({"_type": "ORGANISATION",
                   "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1",
                   "name": name})
        };
        let facts = GrantFacts::of(
            Kind::Organisation,
            &organisation(coded("urn:example:organisation-types", "HOSPITAL")),
        );
        let read: Vec<(FactKind, FactValue)> = facts
            .facts
            .iter()
            .map(|fact| (fact.kind, fact.value.clone()))
            .collect();
        assert_eq!(
            read,
            vec![(
                FactKind::OrganisationType,
                FactValue::Code(code("urn:example:organisation-types", "HOSPITAL"))
            )]
        );
        let text = GrantFacts::of(
            Kind::Organisation,
            &organisation(json!({"_type": "DV_TEXT", "value": "Hospital"})),
        );
        assert_eq!(text.facts, Vec::new(), "a name in text names no type");
        let access = GrantFacts::of(
            Kind::Organisation,
            &organisation(coded(TERMINOLOGY, "HOSPITAL")),
        );
        assert_eq!(access.facts, Vec::new(), "an access code names no type");
    }

    #[test]
    fn a_persons_memberships_are_read_by_their_code_in_any_archetype() {
        let body = json!({"_type": "PERSON", "relationships": [
            relationship(MEMBER, TEAM),
            relationship(REPRESENTS, TEAM),
            relationship(MEMBER, "1.2.840.113619")
        ]});
        let facts = GrantFacts::of(Kind::Person, &body);
        let team = Uuid::parse_str(TEAM).expect("a uuid");
        let read: Vec<(i32, FactKind, FactValue)> = facts
            .facts
            .iter()
            .map(|fact| (fact.ordinal, fact.kind, fact.value.clone()))
            .collect();
        assert_eq!(read, vec![(0, FactKind::MemberOf, FactValue::Team(team))]);
        assert!(facts.facts[0].validity.is_some());
        assert_eq!(facts.unread.len(), 1, "{:?}", facts.unread);
        assert!(facts.unread[0].starts_with("relationships[2]"));
    }

    #[test]
    fn a_membership_is_read_by_the_access_code_alone() {
        let body = json!({"_type": "PERSON", "relationships": [
            relationship(("HL7_RoleClass", "MBR"), TEAM),
            relationship(("HL7_RoleClass", "MBR"), "1.2.840.113619"),
            relationship(("HL7_RoleCode", "MBR"), TEAM)
        ]});
        let facts = GrantFacts::of(Kind::Person, &body);
        assert_eq!(facts.facts, Vec::new());
        assert_eq!(facts.unread, Vec::<String>::new());
    }

    #[test]
    fn an_organisations_member_of_is_its_membership_of_a_group() {
        let body = json!({"_type": "ORGANISATION", "relationships": [
            relationship(MEMBER, TEAM),
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
        assert_eq!(facts.unread.len(), 1, "{:?}", facts.unread);
        assert!(facts.unread[0].starts_with("relationships[1]"));
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
            relationship(MEMBER, other),
        ]);
        let swapped = facts(vec![
            relationship(REPRESENTS, TEAM),
            relationship(MEMBER, other),
            relationship(MEMBER, TEAM),
        ]);
        assert!(
            care_changes(&both, &swapped).is_empty(),
            "order and other relationships"
        );
        assert!(
            !care_changes(&both, &both[..1]).is_empty(),
            "a relationship removed"
        );
        assert!(
            !care_changes(&both[..1], &both).is_empty(),
            "a relationship added"
        );
        let mut ended = relationship(MEMBER, TEAM);
        ended["time_validity"]["upper"] = json!({"_type": "DV_DATE", "value": "2026-11-14"});
        ended["time_validity"]["upper_unbounded"] = json!(false);
        let ended = facts(vec![ended, relationship(MEMBER, other)]);
        assert!(
            !care_changes(&both, &ended).is_empty(),
            "a relationship ended"
        );
        let twice = facts(vec![relationship(MEMBER, TEAM), relationship(MEMBER, TEAM)]);
        assert!(
            !care_changes(&twice, &both[..1]).is_empty(),
            "a repeat removed"
        );
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
    fn every_kind_reads_back_from_its_code() {
        for kind in FactKind::ALL {
            assert_eq!(FactKind::of(kind.code()), Some(kind));
        }
        assert_eq!(FactKind::of("represents"), None);
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

    #[test]
    fn a_groups_membership_names_the_organisation_it_belongs_to() {
        let mut of_organisation = relationship(MEMBER, TEAM);
        of_organisation["target"]["type"] = json!("ORGANISATION");
        let mut of_group = relationship(MEMBER, TEAM);
        of_group["target"]["type"] = json!("GROUP");
        let body = json!({"_type": "GROUP", "relationships": [of_organisation, of_group]});
        let facts = GrantFacts::of(Kind::Group, &body);
        let organisation = Uuid::parse_str(TEAM).expect("a uuid");
        let read: Vec<(FactKind, FactValue)> = facts
            .facts
            .iter()
            .map(|fact| (fact.kind, fact.value.clone()))
            .collect();
        assert_eq!(
            read,
            vec![(FactKind::MemberOf, FactValue::Team(organisation))]
        );
        assert_eq!(facts.unread.len(), 1, "{:?}", facts.unread);
        assert!(
            facts.unread[0].contains("ORGANISATION the GROUP belongs to"),
            "{:?}",
            facts.unread
        );
    }
}
