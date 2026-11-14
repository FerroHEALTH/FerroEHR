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

use crate::access_eval::class::{Class, Classes};
use crate::access_eval::procedure::{
    Asked, Caller, Ehr, Emergency, Judged, Outcome, Policy, Verdict, decide,
};
use crate::access_eval::scope::{CategoryIndex, Item, Scopes};
use crate::ids::EhrId;
use crate::party_binding::grants::caller::CareTeam;
use crate::party_binding::grants::live::LiveGrants;
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::storage::access_projection::Revision;

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
    recorded: Recorded,
    stale: Option<Arc<StaleCopies>>,
}

/// The EHRs whose copy on this replica a read found older than the row in
/// force: the copy drops each before its next read of the EHR.
///
/// A replica learns of a change committed on another one by its
/// notification, which arrives after the commit. A read in between compares
/// the revision its verdict was judged on with the row it reads, refuses on
/// a mismatch, and marks the copy here, so the next request reads the row.
/// No openEHR spec governs this: our own design (#5589).
#[derive(Debug, Default)]
pub struct StaleCopies(Mutex<BTreeSet<EhrId>>);

impl StaleCopies {
    /// Marks the copy of `ehr_id` stale.
    pub fn mark(&self, ehr_id: EhrId) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(ehr_id);
    }

    /// The EHRs marked since the last call, which are no longer marked.
    #[must_use]
    pub fn take(&self) -> BTreeSet<EhrId> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
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
            recorded: Recorded::default(),
            stale: None,
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

    /// The verdict for an EHR with `settings`, the caller `cared` for its
    /// subject or not.
    #[must_use]
    pub fn judge(&self, settings: Option<&EhrAccessSettings>, cared: bool) -> Verdict {
        let labels = match settings {
            Some(EhrAccessSettings::Directives(settings)) => {
                settings.labels.as_deref().unwrap_or_default()
            }
            _ => &[],
        };
        let scopes = Scopes::new(self.categories.clone(), labels, self.classifications());
        self.decide(settings, cared, &scopes)
    }

    /// The verdict for every EHR of `class`, the caller `cared` for its
    /// subject or not, its facts left to the query.
    #[must_use]
    pub fn judge_class(&self, class: &Class, cared: bool) -> Verdict {
        let scopes = Scopes::of_class(
            self.categories.clone(),
            Arc::clone(&class.layout),
            self.classifications(),
        );
        self.decide((*class.settings).as_ref(), cared, &scopes)
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
        cared: bool,
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
            ehr: Ehr {
                settings,
                server_default: self.server_default,
                cared,
            },
            scopes,
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
        let judged = self.judgement_of(entry, self.cares_for(entry));
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

    /// The judgement of an EHR as `entry` reads it, the caller `cared` for
    /// its subject or not.
    fn judgement_of(&self, entry: &EhrEntry, cared: bool) -> Judgement {
        Judgement {
            verdict: Arc::new(self.judge((*entry.settings).as_ref(), cared)),
            settings: Arc::clone(&entry.settings),
            version: entry.version.clone(),
            revision: entry.revision,
        }
    }

    /// Judges `ehr_id`, an EHR with no settings a query of this request
    /// served, by the default, and keeps the verdict for the rest of the
    /// request.
    ///
    /// Its subject is not known here. A row of it was served, so the caller
    /// cares for its subject when only the verdict with a care relationship
    /// admits anything; when both verdicts admit, care decided nothing and no
    /// care ground is named. Our own design (#5581).
    pub fn judge_unsettled(&self, ehr_id: EhrId) {
        let entry = EhrEntry {
            settings: Arc::new(None),
            subject: None,
            version: None,
            revision: Revision::default(),
        };
        let cared = self.judge(None, true).admits_any() && !self.judge(None, false).admits_any();
        let judged = self.judgement_of(&entry, cared);
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
            verdict.outcome(item)
        } else {
            tracing::error!(%ehr_id, "an EHR this request did not judge is withheld");
            Outcome::Absent(Refusal::Restricted)
        }
    }

    /// Whether this request judged `ehr_id` on a revision of its settings
    /// row other than `seen`, the revision a read found in force; such a
    /// copy is marked stale. An EHR the request has not judged is not.
    #[must_use]
    pub fn stale(&self, ehr_id: EhrId, seen: Revision) -> bool {
        let Some(judged) = self.judgement(ehr_id) else {
            return false;
        };
        if judged.revision == seen {
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

    /// The condition a query applies, over every EHR of a class among
    /// `classes`, an EHR held closed, and the default for an EHR with no
    /// settings, the caller caring for the subjects `cared`. An EHR of a
    /// class this replica does not hold is withheld.
    #[must_use]
    pub fn condition(&self, classes: &Classes, cared: Vec<String>) -> AccessCondition {
        let pair = |settings: Option<&EhrAccessSettings>| VerdictPair {
            cared: self.judge(settings, true).bare(),
            uncared: self.judge(settings, false).bare(),
        };
        let mut groups: BTreeMap<VerdictPair, Vec<i64>> = BTreeMap::new();
        for (id, class) in &classes.classes {
            let judged = VerdictPair {
                cared: self.judge_class(class, true).bare(),
                uncared: self.judge_class(class, false).bare(),
            };
            groups.entry(judged).or_default().push(*id);
        }
        let mut parties: Vec<String> = self.caller.acts_for.clone();
        parties.extend(self.caller.party.clone());
        parties.sort_unstable();
        parties.dedup();
        let facts = CallerKeys {
            now: self.now,
            parties: parties.into(),
            keeper: self.caller.settings.party().map(str::to_owned),
        };
        AccessCondition::new(
            groups,
            pair(Some(&EhrAccessSettings::HeldClosed)),
            pair(None),
            cared,
            facts,
        )
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
    /// The subjects the caller has a live care relationship to.
    pub cared: Arc<[String]>,
    /// What the facts of a class are read against.
    pub facts: CallerKeys,
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
}

impl From<ConditionGroups> for AccessCondition {
    fn from(groups: ConditionGroups) -> Self {
        Self {
            groups: Some(Arc::new(groups)),
        }
    }
}

impl AccessCondition {
    /// The condition of the classes `groups`, `closed` for an EHR held
    /// closed and `default` for an EHR with no settings.
    fn new(
        groups: BTreeMap<VerdictPair, Vec<i64>>,
        closed: VerdictPair,
        default: VerdictPair,
        cared: Vec<String>,
        facts: CallerKeys,
    ) -> Self {
        let default_admits = default.admits_all();
        let groups: Vec<(Vec<i64>, Arc<VerdictPair>)> = groups
            .into_iter()
            .map(|(pair, ids)| (ids, Arc::new(pair)))
            .collect();
        ConditionGroups {
            groups,
            closed: Arc::new(closed),
            default: (!default_admits).then(|| Arc::new(default)),
            cared: cared.into(),
            facts,
        }
        .into()
    }

    /// The verdicts of a condition that withholds something; `None` when it
    /// admits every row. A condition that admits every row of every class
    /// it knows still withholds the rows of a class it does not.
    #[must_use]
    pub fn groups(&self) -> Option<&ConditionGroups> {
        self.groups.as_deref()
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

/// Whether `item` of `ehr_id`, read beside the revision `seen` of the EHR's
/// settings row, may leave the store for the running request: always outside
/// a request, and for an object of no EHR. A verdict judged on another
/// revision admits nothing ([`RequestAccess::stale`]).
#[must_use]
pub fn admits(ehr_id: Option<EhrId>, seen: Revision, item: &Item<'_>) -> bool {
    match (current(), ehr_id) {
        (Some(access), Some(ehr_id)) => {
            !access.stale(ehr_id, seen) && access.outcome(ehr_id, item) == Outcome::Admit
        }
        _ => true,
    }
}

/// Whether the running request judged `ehr_id` on a revision of its settings
/// row other than `seen`; `false` outside a request.
#[must_use]
pub fn stale(ehr_id: EhrId, seen: Revision) -> bool {
    current().is_some_and(|access| access.stale(ehr_id, seen))
}
