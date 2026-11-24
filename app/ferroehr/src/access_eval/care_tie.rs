// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care relationships by which a caller holds one subject, each with its
//! path and role, and what an actor scope's `care_roles` and `care_path` ask
//! of them.
//!
//! A caller holds a relationship by the `direct` path when its target is the
//! caller's `PERSON` or a `ROLE` it performs and its basis is `assigned`, by
//! the `team` path when its target is a care team or unit the caller is a
//! member of, and by the `self_declared` path when its target is the caller's
//! `PERSON` and its basis is `self_declared` (draft access control decision
//! procedure, Holding a Care Relationship). An actor scope with
//! `relationship` `care` applies to a caller holding a live relationship whose
//! role is one of its `care_roles` by the path its `care_path` names; void
//! means any role and the paths the policy turns on. A caller holds a tie by
//! the `attested` path when its verified credential attests a treatment
//! context under an agreement the policy names, in the role of the
//! `care_role` value set it attests or else `PART`, and no stored
//! relationship backs it (#6078). No role implies another and no path
//! another.
//!
//! Where several relationships admit an access the access log names one:
//! `direct` before `team` before `self_declared` before `attested`, then by
//! the order of the roles in the scope that matched (decision procedure,
//! Holding a Care Relationship). The procedure and its order are the draft's;
//! reading them over the caller facts is our own design (#6075).

use std::sync::Arc;

use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::care::CarePath;
use openehr_rm::draft::access_control::terminology::{
    ForeignCode, ForeignKind, Relationship, Term,
};
use uuid::Uuid;

use crate::party_binding::grants::caller::care_of::{
    CareOf, CarerKind, Cover, Episode, ProviderKey,
};
use crate::party_binding::grants::caller::facts::CallerFacts;
use crate::versioning::episode::EpisodeKey;

/// One live care relationship by which the caller holds the subject.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tie {
    /// How the relationship reaches the caller.
    pub path: CarePath,
    /// The relationship's `care_role`.
    pub role: ForeignCode,
    /// What the access log names of it; `None` where a query read the tie
    /// without the relationship it rests on.
    pub grant: Option<Arc<Grant>>,
}

/// What the access log names of the relationship an access rested on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Grant {
    /// The `PARTY_RELATIONSHIP` container that holds it.
    pub relationship: Option<Uuid>,
    /// The `OBJECT_VERSION_ID` it was read from.
    pub version: Option<String>,
    /// The carer it targets: for the `team` path, the care team or unit.
    pub carer: Uuid,
    /// What the carer is; `None` where no store named it.
    pub carer_kind: Option<CarerKind>,
    /// The provider in whose care it is.
    pub provider: ProviderKey,
    /// Whether it had ended and the grace after its end admitted the access.
    pub in_grace: bool,
    /// The carer it covers for, with its kind.
    pub covers_for: Option<Cover>,
    /// The episode of care it is scoped to.
    pub episode: Option<Arc<Episode>>,
}

impl Tie {
    /// The tie by which `care` reaches a caller with `facts` and the
    /// principals `held` (sorted) on `day`; `None` when it reaches none of
    /// them by a path.
    #[must_use]
    pub fn of(
        care: &CareOf,
        facts: &CallerFacts,
        held: &[Uuid],
        day: jiff::civil::Date,
    ) -> Option<Self> {
        let carer = care.carer();
        if held.binary_search(&carer).is_err() {
            return None;
        }
        let person = facts.party() == Some(carer);
        let path = if care.self_declared() {
            person.then_some(CarePath::SelfDeclared)?
        } else if person || facts.performed().binary_search(&carer).is_ok() {
            CarePath::Direct
        } else if facts.care_teams().iter().any(|team| team.team() == carer) {
            CarePath::Team
        } else {
            return None;
        };
        Some(Self {
            path,
            role: care.role().clone(),
            grant: Some(Arc::new(Grant {
                relationship: care.relationship(),
                version: care.version().map(str::to_owned),
                carer,
                carer_kind: care.carer_kind(),
                provider: care.provider().clone(),
                in_grace: care.in_grace_on(day),
                covers_for: care.covers_for().cloned(),
                episode: care.episode().cloned(),
            })),
        })
    }

    /// The episode the tie is scoped to; `None` when it names none, or a
    /// query read it without the relationship.
    #[must_use]
    pub fn episode(&self) -> Option<EpisodeKey> {
        self.grant
            .as_ref()?
            .episode
            .as_ref()
            .map(|episode| episode.key)
    }

    /// The tokens a query's census holds for the tie: its path, and its path
    /// with its role, each part joined by U+001F, which no code holds.
    #[must_use]
    pub fn tokens(&self) -> [String; 2] {
        let path = self.path.code();
        [path.to_owned(), token(path, &self.role)]
    }
}

/// The token of `role` by the path `path`.
fn token(path: &str, role: &ForeignCode) -> String {
    format!("{path}\u{1f}{}\u{1f}{}", role.terminology, role.code)
}

/// The rank of `path` in the order the access log chooses by.
const fn rank(path: CarePath) -> u8 {
    match path {
        CarePath::Direct => 0,
        CarePath::Team => 1,
        CarePath::SelfDeclared => 2,
        CarePath::Attested => 3,
    }
}

/// What one actor scope's `care_roles` and `care_path` ask of a caller's
/// care relationships.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CareNeed {
    roles: Option<Vec<ForeignCode>>,
    path: Option<CarePath>,
}

impl CareNeed {
    /// What `scope` asks beside a live care relationship; `None` when it
    /// names neither care roles nor a care path, so any live relationship
    /// meets it.
    ///
    /// A role or a path that does not read is refused at write (draft BMM
    /// `ACCESS_ACTOR_SCOPE`, `Care_roles_valid`, `Care_path_valid`); one read
    /// here anyway meets no relationship.
    #[must_use]
    pub fn of(scope: &AccessActorScope) -> Option<Self> {
        if scope.care_roles.is_none() && scope.care_path.is_none() {
            return None;
        }
        let roles = scope.care_roles.as_ref().map(|roles| {
            roles
                .iter()
                // NOTE: our own design (#6075), the write check refuses a role that does not read.
                .filter_map(|coded| ForeignCode::read(ForeignKind::CareRole, coded).ok())
                .collect()
        });
        let path = match scope.care_path.as_ref().map(CarePath::read_coded) {
            None => None,
            Some(Ok(path)) => Some(path),
            // NOTE: our own design (#6078), a path that does not read names no role it meets.
            Some(Err(_)) => {
                return Some(Self {
                    roles: Some(Vec::new()),
                    path: None,
                });
            }
        };
        Some(Self { roles, path })
    }

    /// The need of `roles` by `path`.
    #[must_use]
    pub fn new(roles: Option<Vec<ForeignCode>>, path: Option<CarePath>) -> Self {
        Self { roles, path }
    }

    /// The roles it names; `None` for any role.
    #[must_use]
    pub fn roles(&self) -> Option<&[ForeignCode]> {
        self.roles.as_deref()
    }

    /// The path it names; `None` for any path the policy turns on.
    #[must_use]
    pub fn path(&self) -> Option<CarePath> {
        self.path
    }

    /// Whether `tie` meets it: one of its roles, by its path.
    #[must_use]
    pub fn met_by(&self, tie: &Tie) -> bool {
        self.path.is_none_or(|path| path == tie.path)
            && self
                .roles
                .as_ref()
                .is_none_or(|roles| roles.contains(&tie.role))
    }

    /// The position of `role` among its roles; `None` when it names none
    /// or not that one.
    fn position(&self, role: &ForeignCode) -> Option<usize> {
        self.roles.as_ref()?.iter().position(|named| named == role)
    }

    /// The census tokens any of which a tie that meets it holds
    /// ([`Tie::tokens`]); a path no relationship is held by matches none.
    #[must_use]
    pub fn tokens(&self) -> Vec<String> {
        let paths: Vec<CarePath> = match self.path {
            Some(path) => vec![path],
            None => vec![
                CarePath::Direct,
                CarePath::Team,
                CarePath::SelfDeclared,
                CarePath::Attested,
            ],
        };
        match &self.roles {
            None => paths.iter().map(|path| path.code().to_owned()).collect(),
            Some(roles) => paths
                .iter()
                .flat_map(|path| roles.iter().map(|role| token(path.code(), role)))
                .collect(),
        }
    }
}

/// The scopes of `rules` and `directives`, exceptions included, that name a
/// care relationship, in the order they are written: whether any does, and
/// the care roles and paths those that narrow it ask.
#[derive(Debug)]
pub struct Needs {
    /// Whether a scope names a care relationship.
    pub named: bool,
    /// What the scopes that narrow it ask, in order.
    pub needs: Vec<CareNeed>,
}

/// The [`Needs`] of the policy's `rules`, then of the EHR's `directives`.
#[must_use]
pub fn care_needs(rules: &[AccessDirective], directives: &[AccessDirective]) -> Needs {
    let mut needs = Needs {
        named: false,
        needs: Vec::new(),
    };
    for directive in rules.iter().chain(directives) {
        let exceptions = directive.exceptions.as_deref().unwrap_or_default();
        for written in std::iter::once(directive).chain(exceptions) {
            for scope in written.applies_to.as_deref().unwrap_or_default() {
                let care = scope
                    .relationship
                    .as_ref()
                    .is_some_and(|coded| Relationship::read_coded(coded) == Ok(Relationship::Care));
                if care {
                    needs.named = true;
                    needs.needs.extend(CareNeed::of(scope));
                }
            }
        }
    }
    needs
}

/// How a decision reads the caller's care for the subject of one EHR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cared {
    /// The caller holds no live care relationship to the subject.
    No,
    /// The live care relationships by which it holds the subject, known for
    /// one EHR; never empty.
    Ties(Arc<[Tie]>),
    /// It holds one, and which roles and paths it holds is left to a query's
    /// statement.
    Left,
    /// As [`Self::Left`], and a relationship of the caller may name an
    /// episode, so which data it covers is left to the statement too.
    Covering,
}

impl Cared {
    /// The care `ties` read; [`Self::No`] when there are none.
    #[must_use]
    pub fn of(ties: Vec<Tie>) -> Self {
        if ties.is_empty() {
            Self::No
        } else {
            Self::Ties(ties.into())
        }
    }

    /// Whether the caller holds a live care relationship to the subject.
    #[must_use]
    pub fn live(&self) -> bool {
        !matches!(self, Self::No)
    }

    /// The live ties, empty when none is known.
    #[must_use]
    pub fn ties(&self) -> &[Tie] {
        match self {
            Self::Ties(ties) => ties,
            Self::No | Self::Left | Self::Covering => &[],
        }
    }
}

/// The tie the access log names among `ties`.
///
/// `needs` are those of the scopes that read care, in the order the rules
/// and directives name them. Of the ties that meet one of them (all, when
/// none does), it is the first by path, then by the position of its role in
/// the first need that names it.
#[must_use]
pub fn granting<'a>(ties: &'a [Tie], needs: &[CareNeed]) -> Option<&'a Tie> {
    let meeting: Vec<&Tie> = ties
        .iter()
        .filter(|tie| needs.iter().any(|need| need.met_by(tie)))
        .collect();
    let candidates = if meeting.is_empty() {
        ties.iter().collect()
    } else {
        meeting
    };
    candidates.into_iter().min_by_key(|tie| {
        let position = needs
            .iter()
            .filter(|need| need.met_by(tie))
            .find_map(|need| need.position(&tie.role))
            .unwrap_or(usize::MAX);
        (rank(tie.path), position, *tie)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(code: &str) -> ForeignCode {
        ForeignCode {
            terminology: "HL7_ParticipationType".to_owned(),
            code: code.to_owned(),
        }
    }

    fn tie(path: CarePath, role: &str) -> Tie {
        Tie {
            path,
            role: code(role),
            grant: None,
        }
    }

    #[test]
    fn a_need_names_roles_and_a_path_and_no_role_implies_another() {
        let resp_direct = CareNeed::new(Some(vec![code("RESP")]), Some(CarePath::Direct));
        assert!(resp_direct.met_by(&tie(CarePath::Direct, "RESP")));
        assert!(!resp_direct.met_by(&tie(CarePath::Team, "RESP")));
        assert!(!resp_direct.met_by(&tie(CarePath::Direct, "CON")));
        assert!(!resp_direct.met_by(&tie(CarePath::SelfDeclared, "RESP")));
        let any_role = CareNeed::new(None, Some(CarePath::Team));
        assert!(any_role.met_by(&tie(CarePath::Team, "PART")));
        let any_path = CareNeed::new(Some(vec![code("ATND")]), None);
        assert!(any_path.met_by(&tie(CarePath::Team, "ATND")));
        assert!(any_path.met_by(&tie(CarePath::SelfDeclared, "ATND")));
        let attested = CareNeed::new(None, Some(CarePath::Attested));
        assert!(attested.met_by(&tie(CarePath::Attested, "PART")));
        assert!(!attested.met_by(&tie(CarePath::Direct, "PART")));
        assert!(!resp_direct.met_by(&tie(CarePath::Attested, "RESP")));
        assert_eq!(attested.tokens(), vec!["attested".to_owned()]);
        let unread = CareNeed::new(Some(Vec::new()), None);
        assert!(!unread.met_by(&tie(CarePath::Attested, "PART")));
        assert_eq!(unread.tokens(), Vec::<String>::new());
    }

    #[test]
    fn the_tokens_of_a_need_meet_the_tokens_of_a_tie_exactly_when_it_does() {
        let needs = [
            CareNeed::new(
                Some(vec![code("RESP"), code("ATND")]),
                Some(CarePath::Direct),
            ),
            CareNeed::new(None, Some(CarePath::Team)),
            CareNeed::new(Some(vec![code("CON")]), None),
            CareNeed::new(None, Some(CarePath::Attested)),
            CareNeed::new(Some(vec![code("PART")]), None),
        ];
        let ties = [
            tie(CarePath::Direct, "RESP"),
            tie(CarePath::Direct, "CON"),
            tie(CarePath::Team, "PART"),
            tie(CarePath::SelfDeclared, "ATND"),
            tie(CarePath::Attested, "PART"),
        ];
        for need in &needs {
            for tie in &ties {
                let tokens = tie.tokens();
                let meets = need.tokens().iter().any(|token| tokens.contains(token));
                assert_eq!(meets, need.met_by(tie), "{need:?} {tie:?}");
            }
        }
    }

    #[test]
    fn the_log_names_direct_before_team_then_the_order_of_the_rule_roles() {
        let ties = [
            tie(CarePath::Team, "RESP"),
            tie(CarePath::Direct, "SPRF"),
            tie(CarePath::Direct, "ATND"),
            tie(CarePath::Direct, "RESP"),
        ];
        let rule = CareNeed::new(Some(vec![code("ATND"), code("RESP")]), None);
        assert_eq!(
            granting(&ties, std::slice::from_ref(&rule)),
            Some(&tie(CarePath::Direct, "ATND"))
        );
        let team_only = CareNeed::new(None, Some(CarePath::Team));
        assert_eq!(
            granting(&ties, &[team_only]),
            Some(&tie(CarePath::Team, "RESP"))
        );
        assert_eq!(granting(&ties, &[]), Some(&tie(CarePath::Direct, "ATND")));
        assert_eq!(granting(&[], &[rule]), None);
    }
}
