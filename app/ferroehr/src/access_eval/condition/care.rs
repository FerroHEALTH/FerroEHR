// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care step of the access evaluator.
//!
//! It holds the reads of [`RequestAccess`] (the subject references the
//! caller cares for, and whether it cares for the `PERSON` an EHR's subject
//! reference names, #5615, #5634) and the forms in which a query reads care
//! ([`Care`], #5699). A decision care admits is taken only on a copy of the
//! caller facts that holds the store's revisions (#5806). No openEHR spec
//! governs where the subject's party is found or how fresh the copy is: our
//! own design.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use uuid::Uuid;

use super::{EhrEntry, RequestAccess};
use crate::access_eval::care_tie::{Cared, Tie};
use crate::access_eval::procedure::Verdict;
use crate::access_eval::stale::{STALE_POLL, STALE_WAIT};
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::party_binding::grants::caller::care_of::CareOf;
use crate::party_binding::grants::live::GrantsUnavailable;
use crate::party_binding::grants::live::revisions::Revisions;
use crate::storage::access_projection::EhrSubject;

/// Why a care decision could not be taken.
#[derive(Debug, thiserror::Error)]
pub enum CareUnsettled {
    /// The care copy missed and could not read.
    #[error(transparent)]
    Unavailable(#[from] GrantsUnavailable),
    /// The caller facts copy stayed older than the store for the whole wait.
    #[error("the caller facts copy stayed older than the store")]
    Refreshing,
}

impl From<CareUnsettled> for ServiceError {
    /// A copy that cannot be read answers `503`; one that stays older than
    /// the store answers `503` with the `access-refreshing` problem.
    fn from(error: CareUnsettled) -> Self {
        match error {
            CareUnsettled::Unavailable(error) => error.into(),
            CareUnsettled::Refreshing => Self::access_refreshing(),
        }
    }
}

/// Whether a request found the caller facts copy in force, and the
/// principals the caller still holds once the copy was read again.
#[derive(Debug, Default)]
pub(super) struct CareInForce(Mutex<Checked>);

/// What a request's check of the caller facts copy found.
#[derive(Debug, Default, Clone)]
enum Checked {
    /// Not checked yet.
    #[default]
    Unchecked,
    /// The store's revisions were read ahead, alongside the request's other
    /// reads: `None` when the copy held them.
    Read(Option<Revisions>),
    /// The copy held the store's revisions.
    Held,
    /// The copy was older and was read again: the caller's principals in the
    /// copy read, sorted.
    ReadAgain(Vec<Uuid>),
}

impl CareInForce {
    fn get(&self) -> Checked {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set(&self, checked: Checked) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = checked;
    }

    /// Sets `checked` unless the request has checked the copy already.
    fn set_unchecked(&self, checked: Checked) {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*held, Checked::Unchecked) {
            *held = checked;
        }
    }
}

impl RequestAccess {
    /// The subject references of the `PERSON`s the caller has a live care
    /// relationship to, each with the ties by which it holds them
    /// ([`crate::party_binding::grants::live::LiveGrants::cared_for_by`]); a
    /// caller with no principal or no provider reads nothing, and an access
    /// whose purpose a care relationship is no ground for reads nothing.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the care copy misses and cannot read.
    pub async fn cared_subjects(&self) -> Result<Vec<CaredSubject>, GrantsUnavailable> {
        let principals = self.principals_held();
        let Some(grants) = self.grants.as_ref().filter(|_| self.may_hold_care()) else {
            return Ok(Vec::new());
        };
        let Some(day) = self.today() else {
            return Ok(Vec::new());
        };
        grants
            .cared_for_by(&self.caller.facts, &principals, day, &self.policy())
            .await
    }

    /// The ties by which the caller holds a subject whose care relationships
    /// are `relations`: each one live, targeting a principal of the caller by
    /// a path while it acts for its provider, not written by the caller
    /// naming itself ([`CareOf::admits_writer`]), sorted; none when the
    /// access declares a purpose outside the policy's `care_purposes` (draft
    /// decision procedure, Terms and Holding a Care Relationship).
    #[must_use]
    pub fn ties(&self, relations: &[CareOf]) -> Vec<Tie> {
        let policy = self.policy();
        let Some(day) = self
            .today()
            .filter(|_| policy.admits_care(self.purpose.as_deref()))
        else {
            return Vec::new();
        };
        let principals = self.principals_held();
        let mut ties: Vec<Tie> = relations
            .iter()
            .filter(|care| {
                care.live_on(day, policy.grace)
                    && care.admits_writer(&principals, policy.self_declared)
                    && self.caller.facts.acts_for_provider(care.provider())
            })
            .filter_map(|care| Tie::of(care, &self.caller.facts, &principals, day))
            .collect();
        ties.sort();
        ties
    }

    /// The caller's care for a subject whose care relationships are
    /// `relations` ([`Self::ties`]).
    #[must_use]
    pub fn cared(&self, relations: &[CareOf]) -> Cared {
        Cared::of(self.ties(relations))
    }

    /// The caller's care as a query of every EHR reads it: [`Cared::Left`]
    /// when the access may rest on a care relationship, [`Cared::No`] when
    /// it declares a purpose outside the policy's `care_purposes`.
    #[must_use]
    pub fn care_left(&self) -> Cared {
        if self.policy().admits_care(self.purpose.as_deref()) {
            Cared::Left
        } else {
            Cared::No
        }
    }

    /// Whether the caller can hold a care relationship at all: it has a
    /// principal and acts for a provider, so a caller with neither reads no
    /// subject's relationships.
    fn may_hold_care(&self) -> bool {
        !self.caller.facts.providers().is_empty() && !self.principals_held().is_empty()
    }

    /// What a care relationship's target names to reach the caller, sorted
    /// ([`crate::party_binding::grants::caller::facts::CallerFacts::principals`]),
    /// less any the caller was found to have lost when the copy was read
    /// again ([`Self::care_in_force`]).
    #[must_use]
    pub fn principals_held(&self) -> Vec<Uuid> {
        let principals = self.caller.facts.principals();
        match self.care.get() {
            Checked::ReadAgain(held) => principals
                .into_iter()
                .filter(|principal| held.binary_search(principal).is_ok())
                .collect(),
            Checked::Unchecked | Checked::Read(_) | Checked::Held => principals,
        }
    }

    /// Whether the caller facts copy holds the store's revisions, so a
    /// decision care admits may be taken on it: `true` when it did, `false`
    /// when it was older and was read again, after which the caller's care
    /// is read again too. Checked once per request, with one indexed read,
    /// or with the read [`Self::care_ahead`] made beside the request's own.
    /// `beside` runs during that indexed read, and is dropped unrun when the
    /// check needs no read.
    ///
    /// The principals the caller holds in the copy read are those it keeps;
    /// one it gained since the request began waits for the next request,
    /// which fails closed. A copy that stays older for
    /// [`STALE_WAIT`] refuses the request. No openEHR spec governs the
    /// copy: our own design (#5806).
    ///
    /// # Errors
    /// [`CareUnsettled::Unavailable`] when the store does not answer, and
    /// [`CareUnsettled::Refreshing`] once the wait has passed.
    pub async fn care_in_force(
        &self,
        beside: impl Future<Output = ()>,
    ) -> Result<bool, CareUnsettled> {
        let Some(grants) = self.grants.as_ref() else {
            return Ok(true);
        };
        let behind = match self.care.get() {
            Checked::Unchecked => {
                // NOTE: our own design (#5959), `beside` runs during this party read, the decision's
                // one wait on the store, and on no connection of the party pool.
                let (behind, ()) = tokio::join!(grants.behind(), beside);
                behind?
            }
            Checked::Read(behind) => behind,
            Checked::Held | Checked::ReadAgain(_) => return Ok(true),
        };
        let Some(store) = behind else {
            self.care.set(Checked::Held);
            return Ok(true);
        };
        let since = Instant::now();
        loop {
            grants.refresh().await?;
            if grants.holds(store) {
                break;
            }
            if since.elapsed() >= STALE_WAIT {
                count("refused");
                return Err(CareUnsettled::Refreshing);
            }
            tokio::time::sleep(STALE_POLL).await;
        }
        let held = self.caller.facts.party().map_or_else(Vec::new, |party| {
            grants.facts_of(party, self.now).principals()
        });
        self.care.set(Checked::ReadAgain(held));
        count("judged_again");
        Ok(false)
    }

    /// Reads the store's revisions ahead for [`Self::care_in_force`] when the
    /// verdict of `ehr_id` as `entry` reads it is one the caller's care
    /// decides, so a service can run the read alongside its own and the
    /// exact check adds no latency (owner decision on #5891). No openEHR spec
    /// governs this: our own design.
    pub async fn care_ahead(&self, ehr_id: EhrId, entry: &EhrEntry) {
        // NOTE: our own design (#5891), a care read that fails here reads ahead nothing, and the
        // judgement reads it again and answers that read's error.
        if matches!(self.care.get(), Checked::Unchecked)
            && self.care_decided(ehr_id, entry, &Cared::Left).is_some()
            && matches!(self.care_for(entry).await, Ok(care) if care.live())
        {
            self.care_read_ahead().await;
        }
    }

    /// Reads the store's revisions ahead for [`Self::care_in_force`] when the
    /// caller has a principal, for a query whose condition reads care. No openEHR spec governs this: our own design (#5891).
    pub async fn care_read_ahead(&self) {
        let Some(grants) = self.grants.as_ref() else {
            return;
        };
        if !self.may_hold_care() || !matches!(self.care.get(), Checked::Unchecked) {
            return;
        }
        // NOTE: our own design (#5891), a read ahead that fails leaves the check unchecked, and
        // care_in_force reads again and answers that read's error.
        if let Ok(behind) = grants.behind().await {
            self.care.set_unchecked(Checked::Read(behind));
        }
    }

    /// The verdict of `ehr_id` as `entry` reads it with the caller's care
    /// `care` for its subject, when it differs from the verdict without that
    /// care.
    pub(super) fn care_decided(
        &self,
        ehr_id: EhrId,
        entry: &EhrEntry,
        care: &Cared,
    ) -> Option<Verdict> {
        let settings = (*entry.settings).as_ref();
        let (with, without) = (
            self.judge(Some(ehr_id), settings, care),
            self.judge(Some(ehr_id), settings, &Cared::No),
        );
        let decides = (&with.rules, with.otherwise, &with.lifted)
            != (&without.rules, without.otherwise, &without.lifted);
        decides.then_some(with)
    }

    /// Of the EHRs `entries` name, those whose subject the caller has a live
    /// care relationship to, each with the ties by which it holds it, judged
    /// as the care step judges one EHR.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the care copy misses and cannot read.
    pub async fn cared_ehrs(
        &self,
        entries: &[(EhrId, EhrEntry)],
    ) -> Result<Vec<CaredEhr>, GrantsUnavailable> {
        let mut cared = Vec::new();
        for (ehr_id, entry) in entries {
            if let Cared::Ties(ties) = self.care_for(entry).await? {
                cared.push(CaredEhr {
                    ehr: ehr_id.0,
                    ties,
                });
            }
        }
        Ok(cared)
    }

    /// The caller's care for the `PERSON` the subject reference of `entry`
    /// names; a caller with no principal has none, and reads nothing for it.
    pub(super) async fn care_for(&self, entry: &EhrEntry) -> Result<Cared, GrantsUnavailable> {
        let under_care_of = match (&self.grants, &entry.subject) {
            (Some(grants), Some(subject)) if self.may_hold_care() => {
                match grants.person_of(subject).await? {
                    Some(person) => grants.under_care_of(person).await?,
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        };
        Ok(self.cared(&under_care_of))
    }
}

/// A subject reference the caller has a live care relationship to, with the
/// ties by which it holds it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CaredSubject {
    /// The subject reference.
    pub subject: EhrSubject,
    /// The ties, each without the relationship it rests on.
    pub ties: Arc<[Tie]>,
}

/// An EHR whose subject the caller has a live care relationship to, with the
/// ties by which it holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaredEhr {
    /// The EHR.
    pub ehr: Uuid,
    /// The ties; never empty.
    pub ties: Arc<[Tie]>,
}

/// How a condition reads the caller's care for the subject of an EHR.
///
/// A query pinned to EHRs it names reads their care from the evaluator's
/// copy and binds only the EHRs it found cared for; any other query binds
/// the subjects of every patient under the caller's care teams. The service
/// resolves [`Care::Unresolved`] before the statement is built, once it knows
/// which EHRs the query reads (#5699). No openEHR spec governs this: our own
/// design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Care {
    /// Not read yet: the service reads it once it knows the query's EHRs.
    Unresolved,
    /// The subject references of every `PERSON` the caller has a live care
    /// relationship to, each with its ties.
    Census(Arc<[CaredSubject]>),
    /// Of the EHRs the query is pinned to, those whose subject the caller has
    /// a live care relationship to, each with its ties; every other EHR is
    /// uncared.
    Ehrs(Arc<[CaredEhr]>),
    /// The clinical care projection, read by the EHR row's subject digest:
    /// the caller's principals and providers and the day and grace a
    /// relationship is live on, as
    /// [`crate::party_binding::grants::caller::care_of::CareOf::live_on`]
    /// judges it.
    Projected(ProjectedCare),
}

/// What a probe of the clinical care projection binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedCare {
    /// The caller's principals: its `PERSON`, the `ROLE`s it performs and
    /// the care teams and units it is a member of.
    pub principals: Arc<[Uuid]>,
    /// Of [`Self::principals`], those a relationship reaches by the `direct`
    /// path: its `PERSON` and the `ROLE`s it performs; the rest reach by the
    /// `team` path.
    pub direct: Arc<[Uuid]>,
    /// The issuers of the providers the caller acts for, each beside its id
    /// in [`Self::provider_ids`].
    pub provider_issuers: Arc<[String]>,
    /// The ids of the providers the caller acts for.
    pub provider_ids: Arc<[String]>,
    /// The day of the request.
    pub day: jiff::civil::Date,
    /// The grace after a relationship ends, as a PostgreSQL interval of
    /// calendar units only (`P14D`).
    pub grace: String,
    /// Whether the policy turns the `self_declared` path on, so a
    /// self-declared relationship admits its writer.
    pub self_declared: bool,
}

/// Increment `access_care_stale_total{outcome}`.
fn count(outcome: &'static str) {
    crate::telemetry::access_path::metrics()
        .care_stale
        .add(1, &[opentelemetry::KeyValue::new("outcome", outcome)]);
}

#[cfg(test)]
#[path = "care_tests.rs"]
mod tests;
