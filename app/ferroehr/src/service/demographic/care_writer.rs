// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The writer of a care relationship judged at the confidentiality of its
//! subject's record (draft access control, The Decision Procedure, Writing
//! Relationships, item 2): a relationship to a subject whose EHR is marked
//! above `N` is written only by a writer `ACCESS_POLICY.relationship_writers`
//! names for that level, and such a write is recorded for review.
//!
//! The markings live in the clinical domain, so they are read before the commit
//! takes its party connection, each connection given back before the next is
//! taken. The administrator is no exception: the draft names none. Our own
//! design over the draft procedure (#6141).

use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::confidentiality_marking::ConfidentialityMarking;
use openehr_rm::draft::access_control::terminology::{CallerKind, Confidentiality, Term};
use uuid::Uuid;

use crate::access_policy::relationship::{self, WriterVerdict};
use crate::error::ServiceError;
use crate::party_binding::grants::caller::facts::CallerFacts;
use crate::party_binding::grants::capabilities::Capabilities;
use crate::service::FerroEhrService;
use crate::service::ehr::access_rule::party_key;
use crate::storage::access_projection::EhrSubject;
use crate::versioning::tree::Tree;

impl FerroEhrService {
    /// Refuses the care relationship `body` when its subject's record is
    /// marked above `N` and no `relationship_writers` entry of the policy in
    /// force names the caller for that level; the level a write is recorded
    /// for review at, `None` for a write at `N`.
    ///
    /// The policy in force is the one the caller confirmed before
    /// ([`Self::confirm_policy`]). A relationship whose source names no
    /// `PERSON` of this store is left to the commit, which refuses it naming
    /// why.
    ///
    /// # Errors
    /// [`ServiceError::Forbidden`] (`403`) naming the level and the missing
    /// rule; a storage failure of either read.
    pub(super) async fn care_writer_check<'a, T: Tree<'a>>(
        &self,
        body: T,
    ) -> Result<Option<Confidentiality>, ServiceError> {
        let Some(person) = body
            .pointer_text("/source/id/value")
            .and_then(|id| crate::party_binding::derive::container(&id))
        else {
            return Ok(None);
        };
        let level = self.subject_confidentiality(person).await?;
        let in_force = self.access_policy().in_force();
        let capabilities = Capabilities::current();
        let writer = Writer::current(capabilities.facts());
        let verdict = relationship::writer(in_force.policy.as_deref(), level, |scope| {
            writer.named_by(scope)
        });
        if verdict == WriterVerdict::Unrestricted {
            return Ok(None);
        }
        if verdict == WriterVerdict::Refused {
            return Err(ServiceError::not_permitted(format!(
                "the subject's record is marked {}: a care relationship to it is written only \
                 by a writer an ACCESS_POLICY.relationship_writers entry names for {}, and no \
                 entry names this caller (an administrator included) for that level",
                level.code(),
                level.code()
            )));
        }
        Ok(Some(level))
    }

    /// The highest confidentiality of the records of `person`: the marking
    /// of every EHR whose subject reference names it, read exactly, `N` when
    /// none is marked. A stored marking that does not read counts as `V`.
    async fn subject_confidentiality(&self, person: Uuid) -> Result<Confidentiality, ServiceError> {
        let references: Vec<EhrSubject> = self
            .subject_references(&[person])
            .await?
            .into_keys()
            .collect();
        let markings =
            crate::storage::subject_ehrs::markings_of_subjects(&self.pool, &references).await?;
        Ok(markings
            .iter()
            .map(|text| level_of(text))
            .max()
            .unwrap_or(Confidentiality::Normal))
    }
}

/// The level of the stored marking `text`.
// NOTE: our own design, as the step-6 read of a record's marking, a marking that does not read
// is held to the highest level, never to normal.
fn level_of(text: &str) -> Confidentiality {
    openehr_its::json::from_canonical_json::<ConfidentialityMarking>(text)
        .ok()
        .and_then(|marking| Confidentiality::read(&marking.confidentiality).ok())
        .unwrap_or(Confidentiality::VeryRestricted)
}

/// The writer of the running request as an actor scope reads it.
struct Writer<'f> {
    /// The party the caller is bound to.
    party: Option<String>,
    /// The caller's roles and organisation types.
    facts: &'f CallerFacts,
    /// The caller's kind: a system client or a professional.
    kind: CallerKind,
}

impl<'f> Writer<'f> {
    /// The writer of the running request, with `facts`.
    fn current(facts: &'f CallerFacts) -> Self {
        let identity = crate::versioning::committer::current_committer();
        let system = identity.as_ref().is_some_and(|identity| identity.system);
        let party = facts.party().map(|party| party.to_string()).or_else(|| {
            identity
                .as_ref()
                .and_then(|identity| identity.party.as_ref())
                .map(|party| party.party.id().to_owned())
        });
        Self {
            party,
            facts,
            kind: if system {
                CallerKind::System
            } else {
                CallerKind::Professional
            },
        }
    }

    /// Whether `scope` names the writer: every attribute it sets matches,
    /// and a scope naming a relationship to the subject names no writer.
    fn named_by(&self, scope: &AccessActorScope) -> bool {
        let party = scope.party.as_ref().is_none_or(|party| {
            party_key(party).is_some_and(|key| self.party.as_ref() == Some(&key))
        });
        let role = scope
            .role
            .as_ref()
            .is_none_or(|role| self.facts.performs(role));
        let organisation = scope
            .organisation_type
            .as_ref()
            .is_none_or(|kind| self.facts.acts_for_type(kind));
        let kind = scope
            .caller_kind
            .as_ref()
            .is_none_or(|coded| CallerKind::read_coded(coded) == Ok(self.kind));
        party && role && organisation && kind && scope.relationship.is_none()
    }
}
