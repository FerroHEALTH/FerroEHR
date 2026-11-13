// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller facts: what the access evaluator reads about a caller beside
//! its grants, and the care relationships of a subject.
//!
//! A `CAPABILITY` clears a sensitivity, and lets its holder declare a
//! purpose of use, by an element anywhere in its `credentials` named by the
//! code [`SensitivityClearance`] or [`PermittedPurpose`] whose value is a
//! `DV_CODED_TEXT` of the terminology the jurisdiction names (RM demographic
//! `org.openehr.rm.demographic.capability.adoc`: `credentials` are "the
//! qualifications of the performer of the role for this capability"). A
//! `PARTY_RELATIONSHIP` of a `PERSON` whose `name` or `details` carry a code
//! of [`CareRelationship`] is a care relationship
//! (`org.openehr.rm.demographic.party_relationship.adoc`: `source`, `target`,
//! `time_validity`): the professional is `member_of` a care team, and the
//! patient is `under_care_of` a team or a unit, with the admission as its
//! `time_validity`. The care delivery environment, such as a hospital's ADT
//! system, writes them under a [`Operation::WriteCareRelationship`] grant
//! (BASE Architecture Overview, `master07-security.adoc` §The Problem of
//! Roles: the evaluation of "care deliverer" into identities "must be done in
//! each care delivery environment, not in the EHR").
//!
//! [`store`] writes the facts in each commit's transaction and stamps the
//! parties whose facts changed, and [`crate::party_binding::grants::live`]
//! holds them on each replica beside the grants, so a request reads them with
//! no transaction of its own. Nothing is read by archetype: every fact is
//! read by its code. No openEHR spec gives a capability or a relationship an
//! access meaning: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the stored canonical party body \
              the caller facts are derived from"
)]

pub mod store;

use serde_json::Value;
use uuid::Uuid;

use crate::error::{ServiceError, Violation};
use crate::party_binding::BindingMapping;
use crate::party_binding::derive::{Facts, container};
use crate::party_binding::grants::capabilities::Capabilities;
use crate::party_binding::grants::derive::{Bound, GrantFacts, bounds, walk};
use crate::party_binding::grants::operation::Operation;
use crate::party_binding::grants::store::Dates;
use crate::party_binding::policy_mark::{TERMINOLOGY, carries, coded};
use crate::versioning::Kind;
use openehr_rm::draft::access_control::terminology::{
    CareRelationship, ForeignCode, ForeignKind, PermittedPurpose, SensitivityClearance, Term,
};

/// What a caller fact is, by the code it is read by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FactKind {
    /// A sensitivity the capability clears.
    Clearance,
    /// A purpose of use the capability lets its holder declare.
    Purpose,
    /// The professional is a member of the care team.
    MemberOf,
    /// The patient is in the care of the team or unit.
    UnderCareOf,
}

impl FactKind {
    /// Every kind, in contract order.
    const ALL: [Self; 4] = [
        Self::Clearance,
        Self::Purpose,
        Self::MemberOf,
        Self::UnderCareOf,
    ];

    /// The code of [`TERMINOLOGY`] the kind is read by, as the side table
    /// stores it.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Clearance => SensitivityClearance::SensitivityClearance.code(),
            Self::Purpose => PermittedPurpose::PermittedPurpose.code(),
            Self::MemberOf => CareRelationship::MemberOf.code(),
            Self::UnderCareOf => CareRelationship::UnderCareOf.code(),
        }
    }

    /// The kind stored as `code`, `None` for another code.
    pub(crate) fn of(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }
}

/// What one fact names: the `CODE_PHRASE` of a clearance or a purpose, or the
/// container a care relationship targets.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FactValue {
    /// A sensitivity or purpose code of the terminology the jurisdiction
    /// names.
    Code(ForeignCode),
    /// The care team `GROUP` or the organisational unit.
    Team(Uuid),
}

/// One caller fact a body holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CallerFact {
    /// The capability's or the relationship's position.
    pub(crate) ordinal: i32,
    /// What the fact is.
    pub(crate) kind: FactKind,
    /// What it names.
    pub(crate) value: FactValue,
    /// The capability's or the relationship's `time_validity`.
    pub(crate) validity: Option<Value>,
    /// The `Not before` bound of a capability's credentials.
    pub(crate) not_before: Bound,
    /// The `Not after` bound of a capability's credentials.
    pub(crate) not_after: Bound,
}

/// Pushes onto `facts` the clearances and permitted purposes `capability`, at
/// `ordinal` of a `ROLE`, holds, and why one cannot be read.
pub(crate) fn qualifiers(ordinal: i32, capability: &Value, facts: &mut GrantFacts) {
    let Some(credentials) = capability.get("credentials") else {
        return;
    };
    let (not_before, not_after) = bounds(credentials);
    walk(credentials, &mut |node| {
        let (kind, foreign) = match node.get("name").and_then(coded) {
            Some(code) if SensitivityClearance::read(TERMINOLOGY, code).is_ok() => {
                (FactKind::Clearance, ForeignKind::Sensitivity)
            }
            Some(code) if PermittedPurpose::read(TERMINOLOGY, code).is_ok() => {
                (FactKind::Purpose, ForeignKind::Purpose)
            }
            _ => return,
        };
        match foreign_code(foreign, node.get("value")) {
            Ok(code) => facts.facts.push(CallerFact {
                ordinal,
                kind,
                value: FactValue::Code(code),
                validity: capability.get("time_validity").cloned(),
                not_before,
                not_after,
            }),
            Err(why) => facts.unread.push(format!(
                "ROLE.capabilities[{ordinal}].credentials: the {TERMINOLOGY}::{} element {why}",
                kind.code()
            )),
        }
    });
}

/// The foreign code of `value`, the value of a clearance or purpose element:
/// a `DV_CODED_TEXT` of the terminology the jurisdiction names, which no
/// code of [`TERMINOLOGY`] is.
fn foreign_code(kind: ForeignKind, value: Option<&Value>) -> Result<ForeignCode, String> {
    let coded = value
        .filter(|value| value.get("_type").and_then(Value::as_str) == Some("DV_CODED_TEXT"))
        .and_then(|value| openehr_its::json::from_canonical_value(value).ok())
        .ok_or_else(|| format!("holds no DV_CODED_TEXT of the {kind} terminology"))?;
    ForeignCode::read(kind, &coded).map_err(|refused| format!("holds no {kind}: {refused}"))
}

/// Pushes onto `facts` the care relationships `body`, a party of `kind`,
/// holds, and why one cannot be read: a care relationship is a relationship
/// of a `PERSON`, its source, to a party of this store, its target.
pub(crate) fn relationships(kind: Kind, body: &Value, facts: &mut GrantFacts) {
    if kind == Kind::PartyRelationship {
        if let Some(care) = care_kind(body) {
            facts.unread.push(format!(
                "a {TERMINOLOGY}::{} relationship is held in its source PERSON's \
                 relationships, never as a PARTY_RELATIONSHIP of its own",
                care.code()
            ));
        }
        return;
    }
    let held = body.get("relationships").and_then(Value::as_array);
    for (ordinal, relationship) in held.into_iter().flatten().enumerate() {
        let Some(care) = care_kind(relationship) else {
            continue;
        };
        let target = relationship
            .pointer("/target/id/value")
            .and_then(Value::as_str)
            .and_then(container);
        let refusal = match (kind, target) {
            (Kind::Person, Some(team)) => {
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
            (Kind::Person, None) => "targets no party of this store by its container",
            _ => "is a relationship of a PERSON, its source",
        };
        facts.unread.push(format!(
            "relationships[{ordinal}]: a {TERMINOLOGY}::{} relationship {refusal}",
            care.code()
        ));
    }
}

/// The care relationship code `relationship` carries in its `name` or its
/// `details`, the first of [`CareRelationship`] in contract order.
fn care_kind(relationship: &Value) -> Option<FactKind> {
    [FactKind::MemberOf, FactKind::UnderCareOf]
        .into_iter()
        .find(|kind| carries(relationship, kind.code()))
}

/// Refuses the committed body `facts` when a caller fact it carries cannot be
/// read: a clearance or purpose element whose value is no code of the
/// jurisdiction's terminology, and a care relationship that is not a
/// `PERSON`'s or names no target of this store.
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

/// The care relationships of a committed `PERSON` before its commit writes
/// them, which [`CareBefore::check`] compares with what it wrote.
#[derive(Debug)]
pub(crate) struct CareBefore(Option<(Uuid, Vec<String>)>);

impl CareBefore {
    /// The stored care relationships of the party `committed` writes, when
    /// it is a `PERSON`, read on its commit transaction `tx`.
    ///
    /// # Errors
    /// A storage failure.
    pub(crate) async fn read(
        tx: &mut sqlx::PgConnection,
        committed: &crate::versioning::change::Committed,
    ) -> Result<Self, ServiceError> {
        if committed.kind != Kind::Person {
            return Ok(Self(None));
        }
        let person = committed.vo_id.0;
        Ok(Self(Some((person, store::care(tx, person).await?))))
    }

    /// Refuses the commit when it changed the care relationships and its
    /// caller may not ([`check_care_writer`]).
    ///
    /// # Errors
    /// The `403` of [`check_care_writer`] and a storage failure.
    pub(crate) async fn check(
        self,
        tx: &mut sqlx::PgConnection,
        mapping: &BindingMapping,
    ) -> Result<(), ServiceError> {
        let Some((person, before)) = self.0 else {
            return Ok(());
        };
        if store::care(tx, person).await? == before {
            return Ok(());
        }
        check_care_writer(mapping, person)
    }
}

/// Refuses a commit that changed the care relationships of the `PERSON`
/// `person` when its caller holds no [`Operation::WriteCareRelationship`]
/// grant and is no administrator of `mapping`.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming the grant.
fn check_care_writer(mapping: &BindingMapping, person: Uuid) -> Result<(), ServiceError> {
    if Capabilities::current().holds(Operation::WriteCareRelationship)
        || crate::party_binding::protect::administrator(mapping)
    {
        return Ok(());
    }
    Err(ServiceError::not_permitted(format!(
        "this version changes the care relationships of PERSON {person} ({TERMINOLOGY}::{} or \
         {TERMINOLOGY}::{}): only a caller holding the {} grant, such as the care delivery \
         environment's ADT system, or an administrator writes them",
        FactKind::MemberOf.code(),
        FactKind::UnderCareOf.code(),
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

/// What the access evaluator reads about one caller beside its grants.
///
/// They are the sensitivities and purposes the capabilities in force clear
/// and permit, and the care teams the caller is a member of on the request's
/// date.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallerFacts {
    clearances: Vec<ForeignCode>,
    purposes: Vec<ForeignCode>,
    care_teams: Vec<CareTeam>,
}

impl CallerFacts {
    /// The facts holding `clearances`, `purposes` and `care_teams`, each
    /// sorted and distinct.
    pub(crate) fn new(
        mut clearances: Vec<ForeignCode>,
        mut purposes: Vec<ForeignCode>,
        mut care_teams: Vec<CareTeam>,
    ) -> Self {
        for codes in [&mut clearances, &mut purposes] {
            codes.sort();
            codes.dedup();
        }
        care_teams.sort();
        care_teams.dedup();
        Self {
            clearances,
            purposes,
            care_teams,
        }
    }

    /// The sensitivity codes the caller's capabilities in force clear.
    #[must_use]
    pub fn clearances(&self) -> &[ForeignCode] {
        &self.clearances
    }

    /// Whether a capability in force clears `sensitivity`, compared by
    /// terminology id and code.
    #[must_use]
    pub fn clears(&self, sensitivity: &ForeignCode) -> bool {
        self.clearances.binary_search(sensitivity).is_ok()
    }

    /// The purposes of use the caller's capabilities in force let it
    /// declare.
    #[must_use]
    pub fn purposes(&self) -> &[ForeignCode] {
        &self.purposes
    }

    /// Whether a capability in force lets the caller declare `purpose`,
    /// compared by terminology id and code.
    #[must_use]
    pub fn permits(&self, purpose: &ForeignCode) -> bool {
        self.purposes.binary_search(purpose).is_ok()
    }

    /// The care teams the caller is a member of on the request's date.
    #[must_use]
    pub fn care_teams(&self) -> &[CareTeam] {
        &self.care_teams
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

    fn relationship(code: &str, target: &str) -> Value {
        json!({"_type": "PARTY_RELATIONSHIP",
               "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.ward.v4",
               "name": {"_type": "DV_TEXT", "value": "ward"},
               "details": {"_type": "ITEM_TREE", "archetype_node_id": "at2",
                           "items": [element(&json!({"_type": "DV_TEXT", "value": "kind"}),
                                             &coded(TERMINOLOGY, code))]},
               "time_validity": {"_type": "DV_INTERVAL",
                                 "lower": {"_type": "DV_DATE", "value": "2026-11-03"},
                                 "lower_unbounded": false, "upper_unbounded": true},
               "target": {"id": {"_type": "HIER_OBJECT_ID", "value": target}}})
    }

    #[test]
    fn a_capability_clears_and_permits_by_coded_element_anywhere_in_its_credentials() {
        let body = json!({
            "_type": "ROLE", "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-ROLE.psychiatrist.v2",
            "capabilities": [
                {"_type": "CAPABILITY", "credentials": {"items": [
                    {"items": [element(&coded(TERMINOLOGY, "sensitivity_clearance"),
                                       &coded("2.16.840.1.113883.5.25", "PSY"))]},
                    element(&coded(TERMINOLOGY, "permitted_purpose"),
                            &coded("2.16.840.1.113883.5.8", "TREAT")),
                    element(&json!({"_type": "DV_TEXT", "value": "sensitivity_clearance"}),
                            &coded("2.16.840.1.113883.5.25", "HIV"))
                ]}},
                {"_type": "CAPABILITY", "credentials": {"items": [
                    element(&coded(TERMINOLOGY, "sensitivity_clearance"),
                            &coded(TERMINOLOGY, "open")),
                    element(&coded(TERMINOLOGY, "permitted_purpose"),
                            &json!({"_type": "DV_TEXT", "value": "TREAT"}))
                ]}}
            ]
        });
        let facts = GrantFacts::of(Kind::Role, &body);
        let read: Vec<(i32, FactKind, FactValue)> = facts
            .facts
            .iter()
            .map(|fact| (fact.ordinal, fact.kind, fact.value.clone()))
            .collect();
        assert_eq!(
            read,
            vec![
                (
                    0,
                    FactKind::Clearance,
                    FactValue::Code(code("2.16.840.1.113883.5.25", "PSY"))
                ),
                (
                    0,
                    FactKind::Purpose,
                    FactValue::Code(code("2.16.840.1.113883.5.8", "TREAT"))
                ),
            ],
            "an element whose name is no code of the terminology clears nothing"
        );
        assert_eq!(facts.unread.len(), 2, "{:?}", facts.unread);
        assert!(facts.unread[0].contains("sensitivity_clearance"));
        assert!(facts.unread[1].contains("permitted_purpose"));
        assert!(
            facts.capabilities.is_empty(),
            "a clearance grants no operation"
        );
    }

    #[test]
    fn a_persons_care_relationships_are_read_by_their_code_in_any_archetype() {
        let body = json!({"_type": "PERSON", "relationships": [
            relationship("member_of", TEAM),
            relationship("represents", TEAM),
            relationship("under_care_of", TEAM),
            relationship("under_care_of", "1.2.840.113619")
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
    fn a_care_relationship_is_a_persons_and_never_one_of_its_own() {
        let body = json!({"_type": "GROUP", "relationships": [relationship("member_of", TEAM)]});
        let facts = GrantFacts::of(Kind::Group, &body);
        assert_eq!(facts.facts, Vec::new());
        assert_eq!(facts.unread.len(), 1);
        let standalone = GrantFacts::of(
            Kind::PartyRelationship,
            &relationship("under_care_of", TEAM),
        );
        assert_eq!(standalone.unread.len(), 1);
        assert_eq!(
            GrantFacts::of(Kind::PartyRelationship, &relationship("represents", TEAM)).unread,
            Vec::<String>::new()
        );
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
        let facts = CallerFacts::new(
            vec![code("ActCode", "PSY"), code("ActCode", "PSY")],
            vec![code("PurposeOfUse", "TREAT")],
            Vec::new(),
        );
        assert_eq!(facts.clearances().len(), 1);
        assert!(facts.clears(&code("ActCode", "PSY")));
        assert!(!facts.clears(&code("local", "PSY")));
        assert!(facts.permits(&code("PurposeOfUse", "TREAT")));
        assert!(!facts.permits(&code("PurposeOfUse", "HOPERAT")));
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
