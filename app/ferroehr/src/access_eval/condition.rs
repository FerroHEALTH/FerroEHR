// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The evaluator for the running request: who asks and what, held for the
//! request's scope, the verdict of each EHR it reaches, and the condition a
//! query applies to every EHR at once.
//!
//! The protocol adapter publishes a [`RequestAccess`] for every operation
//! but the controller's own; the service reads it where data leave the
//! store: a version read, a contribution, an export, the subject lookup, a
//! write, and a query, whose one SQL statement carries the
//! [`AccessCondition`] so counts and pages see only readable rows. Every fact
//! it reads is held in memory on the replica (the per-EHR copy of the
//! settings, the settings classes, the access policy, the caller facts), so a
//! decision adds no transaction. A query's condition is judged per settings
//! class, never per EHR ([`crate::access_eval::class`]), so what a replica
//! holds does not grow with the EHRs that carry settings. Outside a request,
//! a background task or a test, nothing is published and nothing is
//! withheld. No openEHR spec governs this: our own design (#5513, #5588).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{AccessDefault, OperationClass, Refusal};
use tokio::task_local;
use uuid::Uuid;

use crate::access_eval::care_tie::Cared;
use crate::access_eval::class::Class;
use crate::access_eval::condition::care::{Care, CareUnsettled};
use crate::access_eval::procedure::{
    Asked, Caller, Ehr, Emergency, Judged, Outcome, Policy, Verdict, decide, rules_of,
};
use crate::access_eval::record::LiftedCovers;
use crate::access_eval::scope::{CategoryIndex, Item, Scopes};
use crate::access_eval::stale::StaleCopies;
use crate::ids::EhrId;
use crate::party_binding::grants::live::LiveGrants;
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::storage::access_projection::{EhrSubject, Revision};

task_local! {
    static REQUEST_ACCESS: Arc<RequestAccess>;
}

pub mod care;
mod decisions;
pub mod pinned;
mod query;
mod records;
/// The evaluator of one request: the caller, what it asks, the policy and
/// the category map in force when it began, and each EHR's verdict once
/// judged.
#[derive(Debug)]
pub struct RequestAccess {
    caller: Caller,
    operation: OperationClass,
    purpose: Option<String>,
    emergency: Emergency,
    pub(super) policy: Option<Arc<AccessPolicy>>,
    categories: CategoryIndex,
    server_default: AccessDefault,
    now: jiff::Timestamp,
    grants: Option<Arc<LiveGrants>>,
    verdicts: Mutex<BTreeMap<EhrId, Judgement>>,
    /// The subject each EHR's revision named when a read of this request
    /// found it in the store.
    read_subjects: Mutex<BTreeMap<EhrId, Option<EhrSubject>>>,
    recorded: Recorded,
    stale: Option<Arc<StaleCopies>>,
    pub(super) written: Mutex<BTreeSet<Uuid>>,
    care: care::CareInForce,
    /// The EHRs whose subject a read of this request found to be the
    /// `PERSON` the caller is bound to ([`Self::bind_subject`]).
    records: Mutex<BTreeSet<EhrId>>,
}

/// One EHR as the access evaluator reads it from this replica's copy: its
/// current `EHR_ACCESS` settings, the version that carries them, and the
/// `PERSON` its `EHR_STATUS` names.
#[derive(Debug, Clone)]
pub struct EhrEntry {
    /// The settings; `None` when the EHR's `EHR_ACCESS` carries none.
    pub settings: Arc<Option<EhrAccessSettings>>,
    /// The subject reference its `EHR_STATUS` names, whose `PERSON` the
    /// evaluator finds through [`LiveGrants::person_of`]; `None` when it
    /// names none.
    pub subject: Option<EhrSubject>,
    /// The uid of the `EHR_ACCESS` version the settings were read from, which
    /// the access record names; `None` for an EHR with no settings.
    pub version: Option<String>,
    /// The revision of the settings row the entry was read from.
    pub revision: Revision,
}

/// One EHR's verdict in a request, with what it was judged from.
#[derive(Debug, Clone)]
pub struct Judgement {
    /// The verdict.
    pub verdict: Arc<Verdict>,
    /// The settings it was judged on.
    pub settings: Arc<Option<EhrAccessSettings>>,
    /// The uid of the `EHR_ACCESS` version that carries them; `None` when
    /// it could not be read.
    pub version: Option<String>,
    /// The revision of the settings row they were read from.
    pub revision: Revision,
}

/// What the access record of a request names beside the verdicts: the
/// capabilities in force, the version of the access policy, and the
/// refusals each EHR answered, and the one a request naming no EHR answered.
#[derive(Debug, Default)]
struct Recorded {
    facts: RecordFacts,
    refusals: Mutex<BTreeMap<Option<EhrId>, Noted>>,
    lifted: LiftedCovers,
}

/// The facts of a request the access record names that the decision does
/// not read.
#[derive(Debug, Clone, Default)]
pub struct RecordFacts {
    /// The uid of the `ACCESS_POLICY` version in force; `None` when no
    /// version is committed.
    pub policy_version: Option<String>,
    /// The grants in force: each operation code with the container of the
    /// `ROLE` whose `CAPABILITY` grants it.
    pub capabilities: Vec<(&'static str, Uuid)>,
}

/// A refusal an EHR answered: the code and the directives of the rule that
/// decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Noted {
    /// The refusal code.
    pub refusal: Refusal,
    /// The positions of the directives the deciding rule rests on.
    pub by: Vec<usize>,
}

/// What a request asks, beside who asks it.
#[derive(Debug, Clone)]
pub struct Asking {
    /// The class of the operation.
    pub operation: OperationClass,
    /// The declared purpose of use.
    pub purpose: Option<String>,
    /// What the caller declared in an emergency.
    pub emergency: Emergency,
}

/// What the request reads beside the caller and the ask.
#[derive(Debug, Clone)]
pub struct InForce {
    /// The controller's access policy; `None` when no version is committed.
    pub policy: Option<Arc<AccessPolicy>>,
    /// The deployment's category map.
    pub categories: CategoryIndex,
    /// The disposition of an EHR with no settings.
    pub server_default: AccessDefault,
    /// The moment the request began.
    pub now: jiff::Timestamp,
    /// The caller facts held on this replica, which hold each subject's care
    /// relationships.
    pub grants: Option<Arc<LiveGrants>>,
}

impl RequestAccess {
    /// The evaluator of a request by `caller` asking `asking` under `in_force`.
    #[must_use]
    pub fn new(caller: Caller, asking: Asking, in_force: InForce) -> Self {
        Self {
            caller,
            operation: asking.operation,
            purpose: asking.purpose,
            emergency: asking.emergency,
            policy: in_force.policy,
            categories: in_force.categories,
            server_default: in_force.server_default,
            now: in_force.now,
            grants: in_force.grants,
            verdicts: Mutex::new(BTreeMap::new()),
            read_subjects: Mutex::new(BTreeMap::new()),
            recorded: Recorded::default(),
            stale: None,
            written: Mutex::new(BTreeSet::new()),
            care: care::CareInForce::default(),
            records: Mutex::new(BTreeSet::new()),
        }
    }

    /// This evaluator, marking in `stale` each copy a read finds older than
    /// the row in force.
    #[must_use]
    pub fn with_stale_copies(mut self, stale: Arc<StaleCopies>) -> Self {
        self.stale = Some(stale);
        self
    }

    /// This evaluator, its access record naming `facts`.
    #[must_use]
    pub fn with_record_facts(mut self, facts: RecordFacts) -> Self {
        self.recorded.facts = facts;
        self
    }

    /// The caller.
    #[must_use]
    pub fn caller(&self) -> &Caller {
        &self.caller
    }

    /// What the caller declared in an emergency.
    #[must_use]
    pub fn emergency(&self) -> Emergency {
        self.emergency
    }

    /// The lifted denies that covered an object this request admitted.
    pub(crate) fn lifted_covers(&self) -> &LiftedCovers {
        &self.recorded.lifted
    }

    /// The facts the access record names that the decision does not read.
    #[must_use]
    pub fn record_facts(&self) -> &RecordFacts {
        &self.recorded.facts
    }

    /// The judgement of `ehr_id` in this request.
    #[must_use]
    pub fn judgement(&self, ehr_id: EhrId) -> Option<Judgement> {
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&ehr_id)
            .cloned()
    }

    /// The answer `verdict`, the verdict of `ehr_id`, gives `item`, noting a
    /// refusal for the access record of the request (EHDS Annex II 3.2): the
    /// first refusal of an EHR is the one its record names.
    pub fn note(&self, ehr_id: EhrId, verdict: &Verdict, item: &Item<'_>) -> Outcome {
        self.note_under(Some(ehr_id), verdict, item)
    }

    /// The answer to a request of this caller that names no EHR, such as an
    /// EHR create or a query with no single-EHR scope, noted for the access
    /// record as [`Self::note`] notes one: a confined caller reaches one EHR,
    /// and a request naming none names no EHR of its own
    /// (`refused-not-subject`, or `refused-not-represented` for a
    /// representative, step 0); any other caller is judged on each EHR the
    /// operation reaches.
    pub fn judge_unnamed(&self) -> Outcome {
        // NOTE: our own design (#5606, #5609), the evaluator alone holds a confined caller to
        // its one EHR, so a request that names no EHR is judged here.
        if !self.confined() {
            return Outcome::Admit;
        }
        let verdict = self.judge_as(records::Held::default(), None, &Cared::No);
        self.note_under(None, &verdict, &Item::Ehr)
    }

    /// [`Self::note`] under `ehr_id`, `None` for a request naming no EHR.
    fn note_under(&self, ehr_id: Option<EhrId>, verdict: &Verdict, item: &Item<'_>) -> Outcome {
        let rule = verdict.rule_for(item);
        let outcome = rule.map_or(verdict.otherwise, |rule| rule.then);
        let refusal = match outcome {
            Outcome::Admit => {
                self.recorded.lifted.note(ehr_id, verdict, item);
                return outcome;
            }
            Outcome::Absent(refusal) | Outcome::Refused(refusal) => refusal,
        };
        self.recorded
            .refusals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(ehr_id)
            .or_insert_with(|| Noted {
                refusal,
                by: rule.map(|rule| rule.by.clone()).unwrap_or_default(),
            });
        outcome
    }

    /// The refusal noted for `ehr_id`; with no EHR, the first refusal of the
    /// request, with the EHR it was noted for (`None` for a request naming no
    /// EHR).
    #[must_use]
    pub fn noted(&self, ehr_id: Option<EhrId>) -> Option<(Option<EhrId>, Noted)> {
        let refusals = self
            .recorded
            .refusals
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match ehr_id {
            Some(ehr_id) => refusals
                .get(&Some(ehr_id))
                .map(|noted| (Some(ehr_id), noted.clone())),
            None => refusals
                .iter()
                .next()
                .map(|(ehr_id, noted)| (*ehr_id, noted.clone())),
        }
    }

    /// What the controller requires.
    #[must_use]
    pub fn policy(&self) -> Policy<'_> {
        Policy::of(self.policy.as_deref())
    }

    /// Refuses a caseload for this request where the policy turns it off or
    /// the declared purpose is no care purpose
    /// ([`crate::access_eval::caseload::allowed`]).
    ///
    /// # Errors
    /// [`crate::access_eval::caseload::Refused`].
    pub fn caseload_allowed(&self) -> Result<(), crate::access_eval::caseload::Refused> {
        crate::access_eval::caseload::allowed(self.policy.as_deref(), self.purpose.as_deref())
    }

    /// The namespace this CDR names its parties under; empty with no caller
    /// facts copy.
    #[must_use]
    pub fn party_namespace(&self) -> Arc<str> {
        self.grants
            .as_ref()
            .map_or_else(|| Arc::from(""), |grants| grants.namespace())
    }

    /// The moment the request began.
    #[must_use]
    pub fn now(&self) -> jiff::Timestamp {
        self.now
    }

    /// The day of [`Self::now`], the date care relationships are judged on:
    /// its date in the time zone of the policy's `time_zone`, or in UTC when
    /// that is void (draft decision procedure, Terms). `None` when this
    /// replica knows no zone of that name, which the write check refuses, so
    /// no care relationship is held.
    #[must_use]
    pub fn today(&self) -> Option<jiff::civil::Date> {
        let zone = match self.policy.as_deref().and_then(|p| p.time_zone.as_deref()) {
            None => jiff::tz::TimeZone::UTC,
            Some(name) => match jiff::tz::TimeZone::get(name) {
                Ok(zone) => zone,
                Err(error) => {
                    tracing::error!(%error, "the access policy names a time zone this replica does not know; no care relationship is held");
                    return None;
                }
            },
        };
        Some(self.now.to_zoned(zone).date())
    }

    /// The verdict for an EHR with `settings`, held by the caller as `held`.
    fn judge_as(
        &self,
        held: records::Held,
        settings: Option<&EhrAccessSettings>,
        care: &Cared,
    ) -> Verdict {
        let labels = match settings {
            Some(EhrAccessSettings::Directives(settings)) => {
                settings.labels.as_deref().unwrap_or_default()
            }
            _ => &[],
        };
        let scopes = Scopes::new(self.categories.clone(), labels, self.classifications());
        self.decide(settings, care, held, &scopes)
    }

    /// Whether `ehr_id` is one of the confined caller's own EHRs.
    fn owns(&self, ehr_id: Option<EhrId>) -> bool {
        ehr_id.is_some_and(|ehr_id| self.caller.own_ehrs.contains(&ehr_id))
    }

    /// Whether the caller is confined, whom a query reads only in
    /// [`Caller::own_ehrs`] (`ConditionGroups::confined`).
    fn confined(&self) -> bool {
        self.caller.confined()
    }

    /// The verdict for every EHR of `class`, the caller's care for its
    /// subject read as `care`, its facts left to the query. A subject caller
    /// is judged as on its own EHR, the one EHR the condition confines it to.
    #[must_use]
    pub fn judge_class(&self, class: &Class, care: &Cared) -> Verdict {
        let scopes = Scopes::of_class(
            self.categories.clone(),
            Arc::clone(&class.layout),
            self.classifications(),
        );
        let held = records::Held::of_class(self.confined());
        self.decide((*class.settings).as_ref(), care, held, &scopes)
    }

    /// The controller's classifications.
    fn classifications(&self) -> &[openehr_rm::draft::access_control::access_control::access_classification::AccessClassification]{
        self.policy
            .as_deref()
            .and_then(|policy| policy.classifications.as_deref())
            .unwrap_or_default()
    }

    /// The verdict for an EHR with `settings` read through `scopes`.
    fn decide(
        &self,
        settings: Option<&EhrAccessSettings>,
        care: &Cared,
        held: records::Held,
        scopes: &Scopes,
    ) -> Verdict {
        decide(&Judged {
            caller: &self.caller,
            asked: Asked {
                operation: self.operation,
                purpose: self.purpose.as_deref(),
                emergency: self.emergency,
            },
            policy: self.policy(),
            rules: rules_of(self.policy.as_deref()),
            ehr: Ehr {
                settings,
                server_default: self.server_default,
                care,
                own: held.own,
                subject: held.subject,
            },
            scopes,
            now: self.now,
        })
    }

    /// Judges the EHR `ehr_id` as `entry` reads it, and keeps the verdict for
    /// the rest of the request.
    ///
    /// A verdict care decides is taken only once the caller facts copy is
    /// found to hold the store's revisions ([`Self::care_in_force`]).
    ///
    /// # Errors
    /// [`CareUnsettled`] when the subject's care relationships are not held
    /// and cannot be read, or the copy stays older than the store.
    pub async fn judge_ehr(
        &self,
        ehr_id: EhrId,
        entry: &EhrEntry,
    ) -> Result<Arc<Verdict>, CareUnsettled> {
        self.bind_subject(ehr_id, entry).await?;
        let mut care = self.care_for(entry).await?;
        if care.live()
            && let Some(with_care) = self.care_decided(ehr_id, entry, &care)
        {
            // NOTE: our own design (#5959), a read the request handed over runs during the care
            // check's party read when the verdict that check confirms would use it.
            let beside = crate::access_eval::alongside::beside(with_care.item_dependent());
            if !self.care_in_force(beside).await? {
                care = self.care_for(entry).await?;
            }
        }
        let judged = self.judgement_of(ehr_id, entry, &care);
        let verdict = Arc::clone(&judged.verdict);
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ehr_id, judged);
        Ok(verdict)
    }

    /// The judgement of the EHR `ehr_id` as `entry` reads it, the caller's
    /// care for its subject read as `care`.
    fn judgement_of(&self, ehr_id: EhrId, entry: &EhrEntry, care: &Cared) -> Judgement {
        Judgement {
            verdict: Arc::new(self.judge(Some(ehr_id), (*entry.settings).as_ref(), care)),
            settings: Arc::clone(&entry.settings),
            version: entry.version.clone(),
            revision: entry.revision.clone(),
        }
    }

    /// Judges `ehr_id`, an EHR with no settings a query of this request
    /// served, by the default, and keeps the verdict for the rest of the
    /// request.
    ///
    /// Its subject is not known here. A row of it was served, so the caller
    /// cares for its subject when only the verdict with a care relationship
    /// admits anything; when both verdicts admit, care decided nothing and no
    /// care ground is named. Which roles and paths it holds is not known, so
    /// the care ground names no relationship. Our own design (#5581).
    pub fn judge_unsettled(&self, ehr_id: EhrId) {
        let entry = EhrEntry {
            settings: Arc::new(None),
            subject: None,
            version: None,
            revision: Revision::default(),
        };
        let at = Some(ehr_id);
        let left = self.care_left();
        let cared = self.judge(at, None, &left).admits_any()
            && !self.judge(at, None, &Cared::No).admits_any();
        let care = if cared { left } else { Cared::No };
        let judged = self.judgement_of(ehr_id, &entry, &care);
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ehr_id, judged);
    }

    /// The verdict judged for `ehr_id` in this request.
    #[must_use]
    pub fn verdict(&self, ehr_id: EhrId) -> Option<Arc<Verdict>> {
        self.judgement(ehr_id).map(|judged| judged.verdict)
    }

    /// The answer for `item` of `ehr_id`; an EHR this request never judged
    /// is absent, since no decision is never read as permissive.
    #[must_use]
    pub fn outcome(&self, ehr_id: EhrId, item: &Item<'_>) -> Outcome {
        if let Some(verdict) = self.verdict(ehr_id) {
            let outcome = verdict.outcome(item);
            if outcome == Outcome::Admit {
                self.recorded.lifted.note(Some(ehr_id), &verdict, item);
            }
            outcome
        } else {
            tracing::error!(%ehr_id, "an EHR this request did not judge is withheld");
            Outcome::Absent(Refusal::Restricted)
        }
    }

    /// Whether this request judged `ehr_id` on a revision of its settings
    /// row other than `seen`, the revision a read found in force; such a
    /// copy is marked stale. An EHR the request has not judged is not.
    #[must_use]
    pub fn stale(&self, ehr_id: EhrId, seen: &Revision) -> bool {
        self.read_in_store(ehr_id, seen);
        let Some(judged) = self.judgement(ehr_id) else {
            return false;
        };
        if judged.revision == *seen {
            return false;
        }
        tracing::warn!(
            %ehr_id,
            "a read found the EHR's access settings newer than this replica's copy; refused"
        );
        if let Some(stale) = &self.stale {
            stale.mark(ehr_id);
        }
        true
    }

    /// Notes `seen`, the revision of `ehr_id` a read of this request found
    /// in the store, so its access record names the subject `seen` names.
    pub fn read_in_store(&self, ehr_id: EhrId, seen: &Revision) {
        self.read_subjects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ehr_id, seen.subject.clone());
    }

    /// The subject of `ehr_id` a read of this request found in the store;
    /// `None` when no read found the EHR or its status names no subject.
    #[must_use]
    pub fn subject_read(&self, ehr_id: EhrId) -> Option<EhrSubject> {
        self.read_subjects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&ehr_id)
            .cloned()
            .flatten()
    }
}

/// The verdicts of one EHR for a caller who cares for its subject and for
/// one who does not; the two are equal where no step reads care.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VerdictPair {
    /// With a live care relationship.
    pub cared: Verdict,
    /// Without one.
    pub uncared: Verdict,
}

impl VerdictPair {
    /// Whether every item of the EHR is admitted.
    #[must_use]
    pub fn admits_all(&self) -> bool {
        [&self.cared, &self.uncared]
            .into_iter()
            .all(|v| v.admitted().constant() == Some(true))
    }

    /// Whether no item of the EHR is admitted.
    #[must_use]
    pub fn admits_none(&self) -> bool {
        [&self.cared, &self.uncared]
            .into_iter()
            .all(|v| !v.admits_any())
    }
}

/// The verdicts a restrictive condition holds.
///
/// They are the settings classes grouped by verdict, the verdict of an EHR
/// held closed and of an EHR with no settings, the subjects the caller cares
/// for, and what the classes' facts are read against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionGroups {
    /// The settings classes, by id, grouped by their verdicts.
    pub groups: Vec<(Vec<i64>, Arc<VerdictPair>)>,
    /// The verdicts of an EHR held closed.
    pub closed: Arc<VerdictPair>,
    /// The verdicts of an EHR with no settings; `None` admits every row.
    pub default: Option<Arc<VerdictPair>>,
    /// Which EHRs' subjects the caller has a live care relationship to.
    pub cared: Care,
    /// The namespace this CDR names its parties under, whose references name
    /// a `PERSON` by its container.
    pub subject_namespace: Arc<str>,
    /// What the facts of a class are read against.
    pub facts: CallerKeys,
    /// For a subject caller, the EHRs it reaches: its own, or none; `None`
    /// for every other caller.
    pub confined: Option<Arc<[Uuid]>>,
}

/// What the facts of a class are read against: the moment of the request,
/// the parties the caller is or acts for, and the party it is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerKeys {
    /// The moment the request began.
    pub now: jiff::Timestamp,
    /// The caller's own party and every party it acts for, sorted.
    pub parties: Arc<[String]>,
    /// The party the caller is bound to, which a gate-keeper names.
    pub keeper: Option<String>,
}

/// The access condition of one query; shared, so a query request stays
/// small.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessCondition {
    groups: Option<Arc<ConditionGroups>>,
    /// The verdict of the one EHR a pinned query reads ([`pinned::PinnedEhr`]).
    pinned: Option<Arc<pinned::PinnedEhr>>,
}

/// The evaluator of the running request; `None` outside a request and for
/// the controller's own operations.
#[must_use]
pub fn current() -> Option<Arc<RequestAccess>> {
    REQUEST_ACCESS.try_with(Arc::clone).ok()
}

/// Run `fut` with `access` as the running request's evaluator, and count
/// its decisions when it ends.
pub async fn with_request_access<F>(access: Arc<RequestAccess>, fut: F) -> F::Output
where
    F: Future,
{
    let output = REQUEST_ACCESS.scope(Arc::clone(&access), fut).await;
    access.count_decisions();
    output
}

/// Whether `item` of `ehr_id` may leave the store for the running request.
///
/// The object a write of this request committed leaves for its response
/// ([`RequestAccess::wrote`]). `seen` is the revision of the EHR's settings row the read found beside
/// the item. Always outside a request, and for an object of no EHR; a
/// verdict judged on another revision admits nothing
/// ([`RequestAccess::stale`]).
#[must_use]
pub fn admits(ehr_id: Option<EhrId>, seen: &Revision, item: &Item<'_>) -> bool {
    match (current(), ehr_id) {
        // NOTE: our own design (owner ruling on #5621), a write's response is built from the
        // version that request committed, so the writer's own object is no read of stored data.
        (Some(access), Some(_)) if matches!(item, Item::Object(object) if access.written(object.vo_id)) => {
            true
        }
        (Some(access), Some(ehr_id)) => {
            !access.stale(ehr_id, seen) && access.outcome(ehr_id, item) == Outcome::Admit
        }
        _ => true,
    }
}

/// Whether the running request judged `ehr_id` on a revision of its settings
/// row other than `seen`; `false` outside a request.
#[must_use]
pub fn stale(ehr_id: EhrId, seen: &Revision) -> bool {
    current().is_some_and(|access| access.stale(ehr_id, seen))
}
