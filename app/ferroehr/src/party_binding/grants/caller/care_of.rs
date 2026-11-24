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
use std::sync::Arc;

use openehr_rm::draft::access_control::terminology::care::{CareBasis, CareElement};
use openehr_rm::draft::access_control::terminology::{ForeignCode, ForeignKind, Term};
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use uuid::Uuid;

use super::{CallerFact, FactKind, FactValue};
use crate::party_binding::derive::container;
use crate::party_binding::grants::derive::{Bound, GrantFacts};
use crate::party_binding::grants::store::Dates;
use crate::party_binding::policy_mark::{coded, first};
use crate::versioning::episode::EpisodeKey;
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
    /// The carer it covers for, by the `issuer`, `id` and `type` of its
    /// `covers_for` `DV_IDENTIFIER`, written in the provider's form; `None`
    /// when it names none.
    pub(crate) covers_for: Option<Cover>,
    /// The episode of care it is scoped to; `None` when it names none.
    pub(crate) episode: Option<Episode>,
}

/// The episode of care a care relationship is scoped to: the `issuer` and
/// `id` of its `episode` `DV_IDENTIFIER`, and their [`EpisodeKey`] (draft
/// access control package, Episodes of Care: `_type_` is not compared).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Episode {
    /// The key a placement is matched by.
    pub key: EpisodeKey,
    /// The identifier's `issuer`.
    pub issuer: String,
    /// The identifier's `id`.
    pub id: String,
}

impl Episode {
    /// The episode `issuer` gives the identifier `id`.
    #[must_use]
    pub fn new(issuer: impl Into<String>, id: impl Into<String>) -> Self {
        let (issuer, id) = (issuer.into(), id.into());
        Self {
            key: EpisodeKey::of(&issuer, &id),
            issuer,
            id,
        }
    }
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
    let episode = episode(body).map_err(|why| wrong.push(why)).ok();
    let covers_for = covers_for(body)
        .and_then(|cover| match cover {
            Some(cover) if carer == Some(cover.carer) => {
                Err("its covers_for names its target, which never covers for itself".to_owned())
            }
            cover => Ok(cover.map(|cover| cover.cover)),
        })
        .map_err(|why| wrong.push(why))
        .ok();
    if let (
        true,
        Some(carer),
        Some(kind),
        Some(role),
        Some(provider),
        Some(self_declared),
        Some(covers_for),
        Some(episode),
    ) = (
        wrong.is_empty(),
        carer,
        kind,
        role,
        provider,
        self_declared,
        covers_for,
        episode,
    ) {
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
                covers_for,
                episode,
            }),
            validity: body.get("time_validity").map(Tree::to_value),
            not_before: Bound::Open,
            not_after: Bound::Open,
            about: None,
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
    let value = named_element(body, CareElement::CareRole.code())
        .ok_or_else(|| "it carries no care_role element, which is 1..1".to_owned())?;
    if value.get_text("_type").as_deref() != Some("DV_CODED_TEXT") {
        return Err("its care_role is no DV_CODED_TEXT".to_owned());
    }
    openehr_its::json::from_canonical_value(&value.to_value())
        .map_err(|error| format!("its care_role does not read: {error}"))
}

/// The `provider` of the care relationship `body`: a `DV_IDENTIFIER` of
/// type `ORGANISATION` with an issuer and an id.
pub(crate) fn provider<'a, T: Tree<'a>>(body: T) -> Result<ProviderKey, String> {
    let value = named_element(body, CareElement::Provider.code())
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

/// The carer a care relationship covers for, as the caller fact keeps it:
/// the `issuer` and `id` of its `covers_for` `DV_IDENTIFIER` and the kind its
/// `type` names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cover {
    /// The identifier's `issuer` and `id`.
    pub key: ProviderKey,
    /// What the carer is, as the identifier's `type` names it.
    pub kind: CarerKind,
}

impl Cover {
    /// The carer `key` names, of `kind`.
    #[must_use]
    pub fn new(key: ProviderKey, kind: CarerKind) -> Self {
        Self { key, kind }
    }
}

/// The carer a care relationship covers for, as its `covers_for` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoversFor {
    /// The identifier, as the caller fact stores it.
    pub(crate) cover: Cover,
    /// The container its `id` names.
    pub(crate) carer: Uuid,
}

/// The carer the care relationship `body` covers for: its `covers_for`
/// `DV_IDENTIFIER`, in the provider's form, its `issuer` the namespace of the
/// carer's reference, its `id` the carer's container and its `type` the
/// carer's kind; `None` when it names none (draft access control package,
/// Care Relationships). Whether that container is held, and as that kind, is
/// judged at the commit ([`crate::versioning::party_refs`]).
///
/// # Errors
/// Why it cannot be read: a `covers_for` that is no `DV_IDENTIFIER` with an
/// issuer, a container id and the type of a carer.
pub(crate) fn covers_for<'a, T: Tree<'a>>(body: T) -> Result<Option<CoversFor>, String> {
    let Some(value) = named_element(body, CareElement::CoversFor.code()) else {
        return Ok(None);
    };
    let identifier: DvIdentifier = openehr_its::json::from_canonical_value(&value.to_value())
        .map_err(|error| format!("its covers_for is no DV_IDENTIFIER: {error}"))?;
    let issuer = identifier.issuer.filter(|issuer| !issuer.trim().is_empty());
    let kind = identifier.r#type.as_deref().and_then(CarerKind::of);
    match (issuer, container(&identifier.id), kind) {
        (Some(issuer), Some(carer), Some(kind)) => Ok(Some(CoversFor {
            cover: Cover::new(ProviderKey::new(issuer, identifier.id), kind),
            carer,
        })),
        _ => Err(
            "its covers_for names the carer it replaces by the namespace of its reference as \
             the issuer, its container as the id and its kind (PERSON, ROLE, GROUP or \
             ORGANISATION) as the type"
                .to_owned(),
        ),
    }
}

/// The episode of care the care relationship `body` is scoped to: its
/// `episode` `DV_IDENTIFIER`, with an `issuer` and an `id`; `None` when it
/// names none (draft access control package, Episodes of Care).
///
/// # Errors
/// Why it cannot be read: an `episode` that is no `DV_IDENTIFIER` with a
/// non-blank issuer and id.
pub(crate) fn episode<'a, T: Tree<'a>>(body: T) -> Result<Option<Episode>, String> {
    let Some(value) = named_element(body, CareElement::Episode.code()) else {
        return Ok(None);
    };
    let identifier: DvIdentifier = openehr_its::json::from_canonical_value(&value.to_value())
        .map_err(|error| format!("its episode is no DV_IDENTIFIER: {error}"))?;
    match identifier.issuer.filter(|issuer| !issuer.trim().is_empty()) {
        Some(issuer) if !identifier.id.trim().is_empty() => {
            Ok(Some(Episode::new(issuer, identifier.id)))
        }
        _ => Err("its episode names the episode of care by an issuer and an id".to_owned()),
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
    let Some(value) = named_element(body, CareElement::Basis.code()) else {
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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CareOf {
    carer: Uuid,
    dates: Dates,
    role: ForeignCode,
    provider: ProviderKey,
    writer: Option<Uuid>,
    self_declared: bool,
    held: Option<Held>,
    covers_for: Option<Cover>,
    episode: Option<Arc<Episode>>,
}

/// Where a care relationship is stored: its own `PARTY_RELATIONSHIP`
/// container and the version its fact was read from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Held {
    relationship: Uuid,
    version: Option<String>,
    carer_kind: CarerKind,
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
            held: None,
            covers_for: None,
            episode: None,
        }
    }

    /// This relationship scoped to the episode `episode`; `None` when it
    /// names none.
    #[must_use]
    pub fn in_episode(mut self, episode: Option<Arc<Episode>>) -> Self {
        self.episode = episode;
        self
    }

    /// The episode of care it is scoped to; `None` when it names none.
    #[must_use]
    pub fn episode(&self) -> Option<&Arc<Episode>> {
        self.episode.as_ref()
    }

    /// This relationship as stored in the `PARTY_RELATIONSHIP` container
    /// `relationship`, read from its version `version`, its target of
    /// `carer_kind`, covering for `covers_for`.
    #[must_use]
    pub fn stored_as(
        mut self,
        (relationship, version): (Uuid, Option<String>),
        carer_kind: CarerKind,
        covers_for: Option<Cover>,
    ) -> Self {
        self.held = Some(Held {
            relationship,
            version,
            carer_kind,
        });
        self.covers_for = covers_for;
        self
    }

    /// The `PARTY_RELATIONSHIP` container that holds it; `None` for one no
    /// store named.
    #[must_use]
    pub fn relationship(&self) -> Option<Uuid> {
        self.held.as_ref().map(|held| held.relationship)
    }

    /// The `OBJECT_VERSION_ID` of the version it was read from; `None` when
    /// no store named one.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.held.as_ref().and_then(|held| held.version.as_deref())
    }

    /// What its target is; `None` for one no store named.
    #[must_use]
    pub fn carer_kind(&self) -> Option<CarerKind> {
        self.held.as_ref().map(|held| held.carer_kind)
    }

    /// The carer it covers for; `None` when it names none.
    #[must_use]
    pub fn covers_for(&self) -> Option<&Cover> {
        self.covers_for.as_ref()
    }

    /// Whether it admits access on `day` only through the grace after its
    /// end: it ended on or before `day`.
    #[must_use]
    pub fn in_grace_on(&self, day: jiff::civil::Date) -> bool {
        self.dates.upper.is_some_and(|upper| upper <= day)
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
            self.held
                .as_ref()
                .and_then(|held| held.version.as_ref())
                .map_or(0, String::capacity),
            self.covers_for.as_ref().map_or(0, |covers| {
                covers
                    .key
                    .issuer
                    .capacity()
                    .saturating_add(covers.key.id.capacity())
            }),
            self.episode.as_ref().map_or(0, |episode| {
                size_of::<Episode>()
                    .saturating_add(episode.issuer.capacity())
                    .saturating_add(episode.id.capacity())
            }),
        ]
        .into_iter()
        .fold(0, usize::saturating_add)
    }
}

#[cfg(test)]
#[path = "care_of_tests.rs"]
mod tests;
