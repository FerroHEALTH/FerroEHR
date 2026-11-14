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
use openehr_rm::draft::access_control::terminology::{AccessDefault, OperationClass};
use tokio::task_local;
use uuid::Uuid;

use crate::access_eval::procedure::{
    Asked, Caller, Ehr, Emergency, Judged, Outcome, Policy, Verdict, decide,
};
use crate::access_eval::scope::{CategoryIndex, Item, Scopes};
use crate::ids::EhrId;
use crate::party_binding::grants::caller::CareTeam;
use crate::party_binding::grants::live::LiveGrants;
use crate::service::ehr::access_settings::EhrAccessSettings;

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
    verdicts: Mutex<BTreeMap<EhrId, Arc<Verdict>>>,
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
        }
    }

    /// The caller.
    #[must_use]
    pub fn caller(&self) -> &Caller {
        &self.caller
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

    /// The verdict for an EHR with `settings`, the caller `cared` for its
    /// subject or not.
    #[must_use]
    pub fn judge(&self, settings: Option<&EhrAccessSettings>, cared: bool) -> Verdict {
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

    /// Judges the EHR `ehr_id` with `settings` whose subject is the `PERSON`
    /// `subject`, and keeps the verdict for the rest of the request.
    pub fn judge_ehr(
        &self,
        ehr_id: EhrId,
        settings: &Option<EhrAccessSettings>,
        subject: Option<Uuid>,
    ) -> Arc<Verdict> {
        let under_care_of = match (&self.grants, subject) {
            (Some(grants), Some(subject)) => grants.under_care_of(subject),
            _ => Vec::new(),
        };
        let verdict = Arc::new(self.judge(settings.as_ref(), self.cared(&under_care_of)));
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ehr_id, Arc::clone(&verdict));
        verdict
    }

    /// The verdict judged for `ehr_id` in this request.
    #[must_use]
    pub fn verdict(&self, ehr_id: EhrId) -> Option<Arc<Verdict>> {
        self.verdicts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&ehr_id)
            .cloned()
    }

    /// The answer for `item` of `ehr_id`; an EHR this request never judged
    /// is absent, since no decision is never read as permissive.
    #[must_use]
    pub fn outcome(&self, ehr_id: EhrId, item: &Item<'_>) -> Outcome {
        match self.verdict(ehr_id) {
            Some(verdict) => verdict.outcome(item),
            None => {
                tracing::error!(%ehr_id, "an EHR this request did not judge is withheld");
                Outcome::Absent(openehr_rm::draft::access_control::terminology::Refusal::Restricted)
            }
        }
    }

    /// The condition a query applies, over every EHR whose settings are in
    /// `index` and the default for every other, the caller caring for the
    /// subjects `cared`.
    #[must_use]
    pub fn condition(
        &self,
        index: &[(EhrId, EhrAccessSettings)],
        cared: Vec<String>,
    ) -> AccessCondition {
        let pair = |settings: Option<&EhrAccessSettings>| VerdictPair {
            cared: self.judge(settings, true),
            uncared: self.judge(settings, false),
        };
        let mut groups: BTreeMap<VerdictPair, Vec<Uuid>> = BTreeMap::new();
        for (ehr_id, settings) in index {
            groups
                .entry(pair(Some(settings)))
                .or_default()
                .push(ehr_id.0);
        }
        AccessCondition::new(groups, pair(None), cared)
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

/// The access condition of one query: the EHRs grouped by verdict, the
/// verdict of an EHR with no settings, and the subjects the caller cares
/// for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessCondition {
    /// The EHRs with settings, grouped by their verdicts.
    pub groups: Vec<(Vec<Uuid>, Arc<VerdictPair>)>,
    /// The verdicts of an EHR with no settings; `None` admits every row.
    pub default: Option<Arc<VerdictPair>>,
    /// Every EHR with settings.
    pub known: Vec<Uuid>,
    /// The subjects the caller has a live care relationship to.
    pub cared: Arc<Vec<String>>,
}

impl AccessCondition {
    /// The condition admitting every row.
    #[must_use]
    pub fn unrestricted() -> Self {
        Self::default()
    }

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
            return Self::unrestricted();
        }
        Self {
            groups,
            default: (!default_admits).then(|| Arc::new(default)),
            known,
            cared: Arc::new(cared),
        }
    }

    /// Whether the condition admits every row.
    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        self.default.is_none() && self.groups.iter().all(|(_, pair)| pair.admits_all())
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
