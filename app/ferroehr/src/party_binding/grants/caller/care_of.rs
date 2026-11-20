// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A care relationship: a `PARTY_RELATIONSHIP` of its own named
//! `under_care_of`.
//!
//! Its `source` is the subject's `PERSON` and its `target` the carer, a
//! `PERSON`, a `ROLE`, a `GROUP` or an `ORGANISATION` unit, with the elements
//! of the `care relationship element` group in its `details` (draft access
//! control package, Care Relationships).
//!
//! The single relationship is the stored unit, each versioned on its own
//! (draft access control package, The Relationship Store), so a care
//! relationship held in a `PERSON`'s `relationships` is refused, naming the
//! resource that holds it. This reads one into the caller fact the
//! decision takes: its carer, its `care_role` and its `provider`. Why one
//! cannot be read is the refusal of its commit (`422`).

use std::fmt;

use openehr_rm::draft::access_control::terminology::{
    CareBasis, CareElement, ForeignCode, ForeignKind, Term,
};
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use uuid::Uuid;

use super::{CallerFact, FactKind, FactValue};
use crate::party_binding::derive::container;
use crate::party_binding::grants::derive::{Bound, GrantFacts};
use crate::party_binding::grants::store::Dates;
use crate::party_binding::policy_mark::{coded, first};
use crate::versioning::tree::Tree;

/// The resource a care relationship is written at.
pub(crate) const RESOURCE: &str = "/demographic/party_relationship";

/// What the target of a care relationship is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CarerKind {
    /// A professional's `PERSON`.
    Person,
    /// A `ROLE` a professional performs.
    Role,
    /// A care team's `GROUP`.
    Group,
    /// An `ORGANISATION` unit.
    Organisation,
}

impl CarerKind {
    /// Every kind, in contract order.
    pub(crate) const ALL: [Self; 4] = [Self::Person, Self::Role, Self::Group, Self::Organisation];

    /// The RM type of the target, as its `PARTY_REF.type` names it.
    #[must_use]
    pub const fn rm_type(self) -> &'static str {
        match self {
            Self::Person => "PERSON",
            Self::Role => "ROLE",
            Self::Group => "GROUP",
            Self::Organisation => "ORGANISATION",
        }
    }

    /// The kind whose RM type is `rm_type`; `None` for any other.
    #[must_use]
    pub fn of(rm_type: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.rm_type() == rm_type)
    }
}

/// The healthcare provider in whose care a relationship is: the `issuer`
/// and `id` of its `provider` `DV_IDENTIFIER`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderKey {
    /// The identifier's issuer: the namespace of the provider's reference,
    /// or the authority that issued its identifier.
    pub issuer: String,
    /// The identifier.
    pub id: String,
}

impl ProviderKey {
    /// The provider `id` of `issuer`.
    #[must_use]
    pub fn new(issuer: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into(),
            id: id.into(),
        }
    }
}

impl fmt::Display for ProviderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.issuer, self.id)
    }
}

/// What one care relationship states beside its dates: its carer, the
/// carer's kind, its `care_role` and its `provider`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CareFact {
    /// The container of the target.
    pub(crate) carer: Uuid,
    /// What the target is.
    pub(crate) kind: CarerKind,
    /// The part the target plays in the care of the subject.
    pub(crate) role: ForeignCode,
    /// The provider in whose care it is.
    pub(crate) provider: ProviderKey,
    /// Whether its `basis` is `self_declared`: written by the professional
    /// it names.
    pub(crate) self_declared: bool,
}

/// Pushes onto `facts` the care relationship `body`, a `PARTY_RELATIONSHIP`
/// of its own named `under_care_of`, states, and its source as
/// [`GrantFacts::care_source`]; or why it cannot be read.
pub(crate) fn read<'a, T: Tree<'a>>(body: T, facts: &mut GrantFacts) {
    let mut wrong = Vec::new();
    let source = body
        .pointer_text("/source/id/value")
        .and_then(|id| container(&id));
    if body.pointer_text("/source/type").as_deref() != Some(CarerKind::Person.rm_type())
        || source.is_none()
    {
        wrong.push("its source is the subject's PERSON of this store, by its container".to_owned());
    }
    let kind = body
        .pointer_text("/target/type")
        .and_then(|rm_type| CarerKind::of(&rm_type));
    let carer = body
        .pointer_text("/target/id/value")
        .and_then(|id| container(&id));
    if kind.is_none() || carer.is_none() {
        wrong.push(
            "its target is a PERSON, a ROLE, a GROUP or an ORGANISATION of this store, by its \
             container"
                .to_owned(),
        );
    }
    let role = care_role(body).map_err(|why| wrong.push(why)).ok();
    let provider = provider(body).map_err(|why| wrong.push(why)).ok();
    let self_declared = basis(body).map_err(|why| wrong.push(why)).ok();
    if let (true, Some(carer), Some(kind), Some(role), Some(provider), Some(self_declared)) =
        (wrong.is_empty(), carer, kind, role, provider, self_declared)
    {
        facts.care_source = source;
        facts.facts.push(CallerFact {
            ordinal: 0,
            kind: FactKind::UnderCareOf,
            value: FactValue::Care(CareFact {
                carer,
                kind,
                role,
                provider,
                self_declared,
            }),
            validity: body.get("time_validity").map(Tree::to_value),
            not_before: Bound::Open,
            not_after: Bound::Open,
        });
        return;
    }
    facts.unread.extend(
        wrong
            .into_iter()
            .map(|why| format!("an under_care_of relationship: {why}")),
    );
}

/// Why a care relationship held in `relationships[ordinal]` of a party is
/// refused: the single relationship is the stored unit.
pub(crate) fn held_inline(ordinal: usize) -> String {
    format!(
        "relationships[{ordinal}]: a care relationship (under_care_of) is a PARTY_RELATIONSHIP of \
         its own, never one of its source's relationships: write it at {RESOURCE}"
    )
}

/// The `value` of the `ELEMENT` in the `details` of `body` named by the
/// `care relationship element` code `element`.
fn element<'a, T: Tree<'a>>(body: T, element: CareElement) -> Option<T> {
    named_element(body, element.code())
}

/// The `value` of the first `ELEMENT` in the `details` of `body` whose
/// `name` is the access terminology's `code`.
pub(crate) fn named_element<'a, T: Tree<'a>>(body: T, code: &str) -> Option<T> {
    first(body.get("details")?, &mut |item: T| {
        let named = item.get_text("_type").as_deref() == Some("ELEMENT")
            && item.get("name").and_then(coded).as_deref() == Some(code);
        named.then(|| item.get("value")).flatten()
    })
}

/// The `care_role` of the care relationship `body`: its `DV_CODED_TEXT`, of
/// a terminology other than the access terminology, which codes none.
///
/// # Errors
/// Why it cannot be read: no `care_role` element, or one that is no such
/// code.
pub(crate) fn care_role<'a, T: Tree<'a>>(body: T) -> Result<ForeignCode, String> {
    let coded = role_text(body)?;
    ForeignCode::read(ForeignKind::CareRole, &coded).map_err(|error| error.to_string())
}

/// The `care_role` `DV_CODED_TEXT` of the care relationship `body`.
///
/// # Errors
/// As [`care_role`].
pub(crate) fn role_text<'a, T: Tree<'a>>(body: T) -> Result<DvCodedText, String> {
    let value = element(body, CareElement::CareRole)
        .ok_or_else(|| "it carries no care_role element, which is 1..1".to_owned())?;
    if value.get_text("_type").as_deref() != Some("DV_CODED_TEXT") {
        return Err("its care_role is no DV_CODED_TEXT".to_owned());
    }
    openehr_its::json::from_canonical_value(&value.to_value())
        .map_err(|error| format!("its care_role does not read: {error}"))
}

/// The `provider` of the care relationship `body`: a `DV_IDENTIFIER` of
/// type `ORGANISATION` with an issuer and an id.
fn provider<'a, T: Tree<'a>>(body: T) -> Result<ProviderKey, String> {
    let value = element(body, CareElement::Provider)
        .ok_or_else(|| "it carries no provider element, which is 1..1".to_owned())?;
    organisation(value)
}

/// The provider the `provider` element `value` names: a `DV_IDENTIFIER` of
/// type `ORGANISATION` with an issuer and an id.
///
/// # Errors
/// Why it names none.
pub(crate) fn organisation<'a, T: Tree<'a>>(value: T) -> Result<ProviderKey, String> {
    let identifier: DvIdentifier = openehr_its::json::from_canonical_value(&value.to_value())
        .map_err(|error| format!("its provider is no DV_IDENTIFIER: {error}"))?;
    let issuer = identifier.issuer.filter(|issuer| !issuer.trim().is_empty());
    match (issuer, identifier.r#type.as_deref()) {
        (Some(issuer), Some("ORGANISATION")) if !identifier.id.trim().is_empty() => {
            Ok(ProviderKey::new(issuer, identifier.id))
        }
        _ => Err(
            "its provider names an ORGANISATION by an issuer and an id, with the type \
             ORGANISATION"
                .to_owned(),
        ),
    }
}

/// Whether the `basis` of `body` is `self_declared`; `assigned`, the
/// default when absent, is not (draft access control package, Care
/// Relationships). The access policy judges a self-declared one at its
/// commit ([`super::bounds`]).
///
/// # Errors
/// Why it cannot be read: a basis that is no code of the care basis group.
fn basis<'a, T: Tree<'a>>(body: T) -> Result<bool, String> {
    let Some(value) = element(body, CareElement::Basis) else {
        return Ok(false);
    };
    match coded(value).as_deref() {
        Some(code) if code == CareBasis::Assigned.code() => Ok(false),
        Some(code) if code == CareBasis::SelfDeclared.code() => Ok(true),
        _ => Err("its basis is no code of the care basis group".to_owned()),
    }
}

/// One care relationship of a subject: its carer, the dates its
/// `time_validity` holds, its `care_role` and its provider.
///
/// The evaluator reads a subject's from
/// [`crate::party_binding::grants::live::LiveGrants::under_care_of`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CareOf {
    carer: Uuid,
    dates: Dates,
    role: ForeignCode,
    provider: ProviderKey,
    writer: Option<Uuid>,
    self_declared: bool,
}

impl CareOf {
    /// The relationship to `carer` over `dates`, in the role `role` and the
    /// care of `provider`.
    pub(crate) fn new(carer: Uuid, dates: Dates, role: ForeignCode, provider: ProviderKey) -> Self {
        Self {
            carer,
            dates,
            role,
            provider,
            writer: None,
            self_declared: false,
        }
    }

    /// This relationship as written by the party `writer`, the committer's
    /// `PERSON` or client `ACTOR`, and self-declared when `self_declared`.
    #[must_use]
    pub fn written_by(mut self, writer: Option<Uuid>, self_declared: bool) -> Self {
        self.writer = writer;
        self.self_declared = self_declared;
        self
    }

    /// Whether a caller whose principals are `principals` (sorted) may be
    /// admitted on it, as far as its writer goes: never on one it wrote
    /// naming itself, unless it is self-declared and the policy turns that
    /// path on (`self_declared_on`); a self-declared one admits only while
    /// that path is on (draft access control decision procedure, Holding a
    /// Care Relationship and Writing Relationships).
    #[must_use]
    pub fn admits_writer(&self, principals: &[Uuid], self_declared_on: bool) -> bool {
        if self.self_declared {
            return self_declared_on;
        }
        self.writer
            .is_none_or(|writer| principals.binary_search(&writer).is_err())
    }

    /// The relationship to `carer` from the date `from` until the first date
    /// it no longer holds, `until`, each `None` when unbounded, in the role
    /// `role` and the care of `provider`.
    #[must_use]
    pub fn spanning(
        carer: Uuid,
        (from, until): (Option<jiff::civil::Date>, Option<jiff::civil::Date>),
        role: ForeignCode,
        provider: ProviderKey,
    ) -> Self {
        Self::new(
            carer,
            Dates {
                lower: from,
                upper: until,
            },
            role,
            provider,
        )
    }

    /// The container of the carer: a `PERSON`, a `ROLE`, a `GROUP` or an
    /// `ORGANISATION`.
    #[must_use]
    pub fn carer(&self) -> Uuid {
        self.carer
    }

    /// The part the carer plays in the care of the subject.
    #[must_use]
    pub fn role(&self) -> &ForeignCode {
        &self.role
    }

    /// The provider in whose care the relationship is.
    #[must_use]
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }

    /// The party that wrote it, the committer's `PERSON` or client `ACTOR`;
    /// `None` when its committer names none.
    #[must_use]
    pub fn writer(&self) -> Option<Uuid> {
        self.writer
    }

    /// Whether its `basis` is `self_declared`.
    #[must_use]
    pub fn self_declared(&self) -> bool {
        self.self_declared
    }

    /// The first date it holds; `None` when unbounded.
    #[must_use]
    pub fn from(&self) -> Option<jiff::civil::Date> {
        self.dates.lower
    }

    /// The first date it no longer holds, such as the day after a discharge;
    /// `None` when unbounded.
    #[must_use]
    pub fn until(&self) -> Option<jiff::civil::Date> {
        self.dates.upper
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

impl crate::memory::Weigh for CareOf {
    fn heap(&self) -> usize {
        [
            self.role.terminology.capacity(),
            self.role.code.capacity(),
            self.provider.issuer.capacity(),
            self.provider.id.capacity(),
        ]
        .into_iter()
        .fold(0, usize::saturating_add)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::party_binding::grants::caller::writer::care_changes;
    use crate::party_binding::policy_mark::TERMINOLOGY;
    use crate::versioning::Kind;

    const TEAM: &str = "0190a1bc-3a1e-72e4-b7c5-f78a24c01308";

    fn coded(terminology: &str, code: &str) -> Value {
        json!({"_type": "DV_CODED_TEXT", "value": code,
               "defining_code": {"_type": "CODE_PHRASE",
                                 "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
                                 "code_string": code}})
    }

    fn element(name: &Value, value: &Value) -> Value {
        json!({"_type": "ELEMENT", "archetype_node_id": "at1", "name": name, "value": value})
    }

    /// A relationship named `code` of `terminology` from and to the
    /// container `TEAM`.
    fn relationship(terminology: &str, code: &str) -> Value {
        json!({"_type": "PARTY_RELATIONSHIP", "archetype_node_id": "at1",
               "name": coded(terminology, code),
               "source": {"id": {"_type": "HIER_OBJECT_ID", "value": TEAM}, "type": "PERSON"},
               "target": {"id": {"_type": "HIER_OBJECT_ID", "value": TEAM}, "type": "GROUP"}})
    }

    /// A care relationship of its own to `TEAM` in the role `PART`, in the
    /// care of one provider.
    fn care_of() -> Value {
        let mut body = relationship(TERMINOLOGY, "under_care_of");
        body["details"] = json!({"_type": "ITEM_TREE", "archetype_node_id": "at2", "items": [
            element(&coded(TERMINOLOGY, "care_role"), &coded("HL7_ParticipationType", "PART")),
            element(&coded(TERMINOLOGY, "provider"),
                    &json!({"_type": "DV_IDENTIFIER", "issuer": "urn:example:providers",
                            "id": "hospital", "type": "ORGANISATION"}))]});
        body
    }

    #[test]
    fn a_care_relationship_is_one_of_its_own_and_never_a_partys() {
        for (kind, rm) in [
            (Kind::Person, "PERSON"),
            (Kind::Organisation, "ORGANISATION"),
        ] {
            let inline = GrantFacts::of(kind, &json!({"_type": rm, "relationships": [care_of()]}));
            assert_eq!(inline.facts, Vec::new(), "{rm}");
            assert_eq!(inline.unread.len(), 1, "{rm}: {:?}", inline.unread);
            assert!(
                inline.unread[0].contains(RESOURCE),
                "{rm}: {:?}",
                inline.unread
            );
        }
        let standalone = GrantFacts::of(Kind::PartyRelationship, &care_of());
        assert_eq!(standalone.unread, Vec::<String>::new());
        assert_eq!(standalone.care_source, Uuid::parse_str(TEAM).ok());
        assert!(matches!(
            standalone.facts.as_slice(),
            [CallerFact {
                kind: FactKind::UnderCareOf,
                value: FactValue::Care(_),
                ..
            }]
        ));
        let mut bare = care_of();
        bare["details"]["items"] = json!([]);
        let refused = GrantFacts::of(Kind::PartyRelationship, &bare);
        assert_eq!(refused.facts, Vec::new());
        assert_eq!(
            refused.unread.len(),
            2,
            "no role, no provider: {:?}",
            refused.unread
        );
        let membership = relationship("HL7_RoleClass", "MBR");
        assert_eq!(
            GrantFacts::of(Kind::PartyRelationship, &membership)
                .unread
                .len(),
            1,
            "a membership is its member's"
        );
        let mut other_role = care_of();
        other_role["details"]["items"][0]["value"] = coded("HL7_ParticipationType", "RESP");
        let other_role = GrantFacts::of(Kind::PartyRelationship, &other_role).facts;
        assert!(
            !care_changes(&standalone.facts, &other_role).is_empty(),
            "a role changed"
        );
    }

    #[test]
    fn a_relationship_never_admits_its_writer_unless_self_declared_and_switched_on() {
        let (writer, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let assigned = |by: Option<Uuid>| {
            CareOf::spanning(
                writer,
                (None, None),
                ForeignCode {
                    terminology: "HL7_ParticipationType".to_owned(),
                    code: "PART".to_owned(),
                },
                ProviderKey::new("urn:example:providers", "hospital"),
            )
            .written_by(by, false)
        };
        assert!(!assigned(Some(writer)).admits_writer(&[writer], true));
        assert!(assigned(Some(other)).admits_writer(&[writer], false));
        assert!(assigned(None).admits_writer(&[writer], false));
        let declared = assigned(Some(writer)).written_by(Some(writer), true);
        assert!(declared.admits_writer(&[writer], true));
        assert!(!declared.admits_writer(&[writer], false), "the path is off");
    }

    #[test]
    fn a_self_declared_basis_reads() {
        let mut body = care_of();
        let basis = element(
            &coded(TERMINOLOGY, "basis"),
            &coded(TERMINOLOGY, "self_declared"),
        );
        body["details"]["items"]
            .as_array_mut()
            .expect("items")
            .push(basis);
        let facts = GrantFacts::of(Kind::PartyRelationship, &body);
        assert_eq!(facts.unread, Vec::<String>::new());
        assert!(matches!(
            facts.facts.as_slice(),
            [CallerFact {
                value: FactValue::Care(CareFact {
                    self_declared: true,
                    ..
                }),
                ..
            }]
        ));
    }
}
