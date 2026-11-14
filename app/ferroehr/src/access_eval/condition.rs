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
//! settings, the access policy, the caller facts), so a decision adds no
//! transaction. Outside a request, a background task or a test, nothing is
//! published and nothing is withheld. No openEHR spec governs this: our own
//! design (#5513).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{AccessDefault, OperationClass, Refusal};
use tokio::task_local;
use uuid::Uuid;

use crate::access_eval::procedure::{
    Asked, Caller, Ehr, Emergency, Judged, Outcome, Policy, Verdict, decide,
};
use crate::access_eval::scope::{CategoryIndex, Item, Scopes};
use crate::ids::EhrId;
use crate::party_binding::grants::caller::CareTeam;
use crate::party_binding::grants::live::LiveGrants;
use crate::service::ehr::access_settings::{EhrAccessIndex, EhrAccessSettings};

task_local! {
    static REQUEST_ACCESS: Arc<RequestAccess>;
}

/// The evaluator of one request: the caller, what it asks, the policy and
/// the category map in force when it began, and each EHR's verdict once
/// judged.
#[derive(Debug)]
pub struct RequestAccess {
    caller: Caller,
    operation: OperationClass,
    purpose: Option<String>,
    emergency: Emergency,
    policy: Option<Arc<AccessPolicy>>,
    categories: CategoryIndex,
    server_default: AccessDefault,
    now: jiff::Timestamp,
    grants: Option<Arc<LiveGrants>>,
    verdicts: Mutex<BTreeMap<EhrId, Judgement>>,
    reached: Mutex<Option<EhrAccessIndex>>,
    recorded: Recorded,
}

/// One EHR as the access evaluator reads it from this replica's copy: its
/// current `EHR_ACCESS` settings, the version that carries them, and the
/// `PERSON` its `EHR_STATUS` names.
#[derive(Debug, Clone)]
pub struct EhrEntry {
    /// The settings; `None` when the EHR's `EHR_ACCESS` carries none.
    pub settings: Arc<Option<EhrAccessSettings>>,
    /// The subject's `PERSON` container; `None` when the status names no
    /// party of this store.
    pub subject: Option<Uuid>,
    /// The uid of the `EHR_ACCESS` version the settings were read from, which
    /// the access record names; `None` for an EHR with no settings.
    pub version: Option<String>,
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
}

/// What the access record of a request names beside the verdicts: the
/// capabilities in force, the version of the access policy, and the
/// refusals each EHR answered.
#[derive(Debug, Default)]
struct Recorded {
    facts: RecordFacts,
    refusals: Mutex<BTreeMap<EhrId, Noted>>,
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
            reached: Mutex::new(None),
            recorded: Recorded::default(),
        }
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
        let rule = verdict.rule_for(item);
        let outcome = rule.map_or(verdict.otherwise, |rule| rule.then);
        let refusal = match outcome {
            Outcome::Admit => return outcome,
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
    /// request.
    #[must_use]
    pub fn noted(&self, ehr_id: Option<EhrId>) -> Option<(EhrId, Noted)> {
        let refusals = self
            .recorded
            .refusals
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match ehr_id {
            Some(ehr_id) => refusals.get(&ehr_id).map(|noted| (ehr_id, noted.clone())),
            None => refusals
                .iter()
                .next()
                .map(|(ehr_id, noted)| (*ehr_id, noted.clone())),
        }
    }

    /// What the controller requires.
    #[must_use]
    pub fn policy(&self) -> Policy {
        Policy::of(self.policy.as_deref())
    }

    /// The moment the request began.
    #[must_use]
    pub fn now(&self) -> jiff::Timestamp {
        self.now
    }

    /// The date of [`Self::now`] in UTC, the date care relationships are
    /// judged on.
    #[must_use]
    pub fn today(&self) -> jiff::civil::Date {
        self.now.to_zoned(jiff::tz::TimeZone::UTC).date()
    }

    /// Whether the caller has a live care relationship to a subject under
    /// the care of `teams`.
    #[must_use]
    pub fn cared(&self, teams: &[CareTeam]) -> bool {
        let (today, grace) = (self.today(), self.policy().grace);
        teams.iter().any(|team| {
            team.live_on(today, grace)
                && self
                    .caller
                    .facts
                    .care_teams()
                    .iter()
                    .any(|own| own.team() == team.team())
        })
    }

    /// The verdict for the EHR `ehr_id` with `settings`, the caller `cared`
    /// for its subject or not; `None` stands for an EHR that is no subject
    /// caller's own.
    #[must_use]
    pub fn judge(
        &self,
        ehr_id: Option<EhrId>,
        settings: Option<&EhrAccessSettings>,
        cared: bool,
    ) -> Verdict {
        let (labels, classifications) = (
            match settings {
                Some(EhrAccessSettings::Directives(settings)) => {
                    settings.labels.as_deref().unwrap_or_default()
                }
                _ => &[],
            },
            self.policy
                .as_deref()
                .and_then(|policy| policy.classifications.as_deref())
                .unwrap_or_default(),
        );
        let scopes = Scopes::new(self.categories.clone(), labels, classifications);
        decide(&Judged {
            caller: &self.caller,
            asked: Asked {
                operation: self.operation,
                purpose: self.purpose.as_deref(),
                emergency: self.emergency,
            },
            policy: self.policy(),
            ehr: Ehr {
                settings,
                server_default: self.server_default,
                cared,
                own: ehr_id.is_some() && ehr_id == self.caller.own_ehr,
            },
            scopes: &scopes,
            now: self.now,
        })
    }

    /// The subjects the caller has a live care relationship to, by their
    /// `PERSON` container id.
    #[must_use]
    pub fn cared_subjects(&self) -> Vec<String> {
        let teams: Vec<Uuid> = self
            .caller
            .facts
            .care_teams()
            .iter()
            .map(CareTeam::team)
            .collect();
        self.grants.as_ref().map_or_else(Vec::new, |grants| {
            grants
                .cared_for_by(&teams, self.today(), self.policy().grace)
                .iter()
                .map(Uuid::to_string)
                .collect()
        })
    }

    /// Judges the EHR `ehr_id` as `entry` reads it, and keeps the verdict for
    /// the rest of the request.
    pub fn judge_ehr(&self, ehr_id: EhrId, entry: &EhrEntry) -> Arc<Verdict> {
        let judged = self.judgement_of(ehr_id, entry, self.cares_for(entry));
        let verdict = Arc::clone(&judged.verdict);
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ehr_id, judged);
        verdict
    }

    /// Whether the caller has a live care relationship to the subject of
    /// `entry`.
    fn cares_for(&self, entry: &EhrEntry) -> bool {
        let under_care_of = match (&self.grants, entry.subject) {
            (Some(grants), Some(subject)) => grants.under_care_of(subject),
            _ => Vec::new(),
        };
        self.cared(&under_care_of)
    }

    /// The judgement of the EHR `ehr_id` as `entry` reads it, the caller
    /// `cared` for its subject or not.
    fn judgement_of(&self, ehr_id: EhrId, entry: &EhrEntry, cared: bool) -> Judgement {
        Judgement {
            verdict: Arc::new(self.judge(Some(ehr_id), (*entry.settings).as_ref(), cared)),
            settings: Arc::clone(&entry.settings),
            version: entry.version.clone(),
        }
    }

    /// The judgement of `ehr_id`, an EHR a query of this request reached
    /// through its condition, judged as a direct read judges it: from the
    /// entry of the index the condition was built on, or, for an EHR with no
    /// settings, the default; `None` when no query built a condition.
    ///
    /// An EHR with no settings is in no index, so its subject is not known
    /// here. A row of it was served, so the caller cares for its subject
    /// when only the verdict with a care relationship admits anything; when
    /// both verdicts admit, care decided nothing and no care ground is named.
    /// Our own design (#5581).
    pub(super) fn reached_judgement(&self, ehr_id: EhrId) -> Option<Judgement> {
        let index = self
            .reached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        let indexed = index
            .binary_search_by_key(&ehr_id, |(id, _)| *id)
            .ok()
            .and_then(|at| index.get(at));
        if let Some((_, entry)) = indexed {
            return Some(self.judgement_of(ehr_id, entry, self.cares_for(entry)));
        }
        let entry = EhrEntry {
            settings: Arc::new(None),
            subject: None,
            version: None,
        };
        let at = Some(ehr_id);
        let cared =
            self.judge(at, None, true).admits_any() && !self.judge(at, None, false).admits_any();
        Some(self.judgement_of(ehr_id, &entry, cared))
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
            verdict.outcome(item)
        } else {
            tracing::error!(%ehr_id, "an EHR this request did not judge is withheld");
            Outcome::Absent(Refusal::Restricted)
        }
    }

    /// The condition a query applies, over every EHR whose settings are in
    /// `index` and the default for every other, the caller caring for the
    /// subjects `cared`; the access record of each EHR the query serves names
    /// its grounds from `index`.
    #[must_use]
    pub fn condition(&self, index: &EhrAccessIndex, cared: Vec<String>) -> AccessCondition {
        *self.reached.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(index));
        let pair = |ehr_id: Option<EhrId>, settings: Option<&EhrAccessSettings>| VerdictPair {
            cared: self.judge(ehr_id, settings, true).bare(),
            uncared: self.judge(ehr_id, settings, false).bare(),
        };
        let mut groups: BTreeMap<VerdictPair, Vec<Uuid>> = BTreeMap::new();
        for (ehr_id, entry) in index.iter() {
            groups
                .entry(pair(Some(*ehr_id), (*entry.settings).as_ref()))
                .or_default()
                .push(ehr_id.0);
        }
        // NOTE: our own design (#5586), a subject caller's own EHR with no settings is in
        // no index, so it is judged on its own beside the default of every other EHR.
        if let Some(own) = self.caller.own_ehr
            && index.binary_search_by_key(&own, |(id, _)| *id).is_err()
        {
            groups.entry(pair(Some(own), None)).or_default().push(own.0);
        }
        AccessCondition::new(groups, pair(None, None), cared)
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

/// The verdicts a restrictive condition holds: the EHRs grouped by verdict,
/// the verdict of an EHR with no settings, and the subjects the caller cares
/// for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConditionGroups {
    /// The EHRs with settings, grouped by their verdicts.
    pub groups: Vec<(Vec<Uuid>, Arc<VerdictPair>)>,
    /// The verdicts of an EHR with no settings; `None` admits every row.
    pub default: Option<Arc<VerdictPair>>,
    /// Every EHR with settings.
    pub known: Vec<Uuid>,
    /// The subjects the caller has a live care relationship to.
    pub cared: Arc<[String]>,
}

/// The access condition of one query; shared, so a query request stays
/// small.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessCondition {
    groups: Option<Arc<ConditionGroups>>,
}

impl From<ConditionGroups> for AccessCondition {
    fn from(groups: ConditionGroups) -> Self {
        Self {
            groups: Some(Arc::new(groups)),
        }
    }
}

impl AccessCondition {
    /// The condition of `groups`, `default` for every other EHR.
    fn new(
        groups: BTreeMap<VerdictPair, Vec<Uuid>>,
        default: VerdictPair,
        cared: Vec<String>,
    ) -> Self {
        let known = groups.values().flatten().copied().collect::<Vec<_>>();
        let default_admits = default.admits_all();
        let groups: Vec<(Vec<Uuid>, Arc<VerdictPair>)> = groups
            .into_iter()
            .map(|(pair, ids)| (ids, Arc::new(pair)))
            .collect();
        if default_admits && groups.iter().all(|(_, pair)| pair.admits_all()) {
            return Self::default();
        }
        ConditionGroups {
            groups,
            default: (!default_admits).then(|| Arc::new(default)),
            known,
            cared: cared.into(),
        }
        .into()
    }

    /// The verdicts of a condition that withholds something; `None` when it
    /// admits every row.
    #[must_use]
    pub fn groups(&self) -> Option<&ConditionGroups> {
        self.groups.as_deref().filter(|groups| {
            groups.default.is_some() || groups.groups.iter().any(|(_, pair)| !pair.admits_all())
        })
    }
}

/// The evaluator of the running request; `None` outside a request and for
/// the controller's own operations.
#[must_use]
pub fn current() -> Option<Arc<RequestAccess>> {
    REQUEST_ACCESS.try_with(Arc::clone).ok()
}

/// Run `fut` with `access` as the running request's evaluator.
pub async fn with_request_access<F>(access: Arc<RequestAccess>, fut: F) -> F::Output
where
    F: Future,
{
    REQUEST_ACCESS.scope(access, fut).await
}

/// Whether `item` of `ehr_id` may leave the store for the running request:
/// always outside a request, and for an object of no EHR.
#[must_use]
pub fn admits(ehr_id: Option<EhrId>, item: &Item<'_>) -> bool {
    match (current(), ehr_id) {
        (Some(access), Some(ehr_id)) => access.outcome(ehr_id, item) == Outcome::Admit,
        _ => true,
    }
}
