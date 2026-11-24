// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The condition a query applies to every EHR at once: the settings classes
//! grouped by the verdicts this request judges them to, built once per query.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::care::Care;
use super::records::Held;
use super::{AccessCondition, CallerKeys, ConditionGroups, RequestAccess, VerdictPair, pinned};
use crate::access_eval::care_tie::Cared;
use crate::access_eval::class::Classes;
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::telemetry::stage::{self, Stage};

impl RequestAccess {
    /// The condition a query applies, over every EHR of a class among
    /// `classes`, an EHR held closed, and the default for an EHR with no
    /// settings, the caller's care read as `care` says. An EHR of a class
    /// this replica does not hold is withheld.
    #[must_use]
    pub fn condition(&self, classes: &Classes, care: Care) -> AccessCondition {
        self.condition_as(classes, care, &self.care_left())
    }

    /// [`Self::condition`], the caller's care for a subject it holds read as
    /// `left`: [`Cared::Left`], [`Cared::Covering`] or [`Cared::No`].
    #[must_use]
    pub fn condition_as(&self, classes: &Classes, care: Care, left: &Cared) -> AccessCondition {
        let _stage = stage::enter(Stage::Access);
        let held = Held::of_class(self.confined());
        let pair = |settings: Option<&EhrAccessSettings>| VerdictPair {
            cared: self.judge_as(&held, settings, left).bare(),
            uncared: self.judge_as(&held, settings, &Cared::No).bare(),
        };
        let mut groups: BTreeMap<VerdictPair, Vec<i64>> = BTreeMap::new();
        for (id, class) in &classes.classes {
            let judged = VerdictPair {
                cared: self.judge_class(class, left).bare(),
                uncared: self.judge_class(class, &Cared::No).bare(),
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
        let default = pair(None);
        ConditionGroups {
            groups: groups
                .into_iter()
                .map(|(pair, ids)| (ids, Arc::new(pair)))
                .collect(),
            closed: Arc::new(pair(Some(&EhrAccessSettings::HeldClosed))),
            default: (!default.admits_all()).then(|| Arc::new(default)),
            cared: care,
            subject_namespace: self
                .grants
                .as_ref()
                .map_or_else(|| Arc::from(""), |grants| grants.namespace()),
            facts,
            // NOTE: our own design (#5586), a confined caller reads only the EHR its patient
            // context names, so a query of it reaches no other EHR whatever its class.
            confined: held
                .own
                .then(|| self.caller.own_ehrs.iter().map(|own| own.0).collect()),
            conflicts: Arc::from([]),
            episodic: *left == Cared::Covering,
        }
        .into()
    }
}

impl From<ConditionGroups> for AccessCondition {
    fn from(groups: ConditionGroups) -> Self {
        Self {
            groups: Some(Arc::new(groups)),
            pinned: None,
        }
    }
}

impl AccessCondition {
    /// The condition of `groups`, pinned to the verdict `pinned` when one is
    /// given.
    #[must_use]
    pub fn of_parts(
        groups: Option<Arc<ConditionGroups>>,
        pinned: Option<Arc<pinned::PinnedEhr>>,
    ) -> Self {
        Self { groups, pinned }
    }

    /// The condition with `conflicts`, the subject references of the persons
    /// the caller is in conflict with, read as no row of theirs admitted.
    #[must_use]
    pub fn with_conflicts(&self, conflicts: Arc<[(String, String)]>) -> Self {
        Self {
            groups: self.groups.as_deref().map(|condition| {
                Arc::new(ConditionGroups {
                    conflicts,
                    ..condition.clone()
                })
            }),
            pinned: self.pinned.clone(),
        }
    }

    /// Whether a verdict of the condition reads care, so the statement needs
    /// [`ConditionGroups::cared`] resolved.
    #[must_use]
    pub fn reads_care(&self) -> bool {
        self.groups().is_some_and(|condition| {
            std::iter::once(&condition.closed)
                .chain(condition.groups.iter().map(|(_, pair)| pair))
                .chain(condition.default.iter())
                .any(|pair| pair.cared != pair.uncared)
        })
    }

    /// The condition with its care read as `care`.
    #[must_use]
    pub fn with_care(&self, care: Care) -> Self {
        Self {
            groups: self.groups.as_deref().map(|condition| {
                Arc::new(ConditionGroups {
                    cared: care,
                    ..condition.clone()
                })
            }),
            pinned: self.pinned.clone(),
        }
    }

    /// The verdicts of a condition that withholds something; `None` when it
    /// admits every row. A condition that admits every row of every class
    /// it knows still withholds the rows of a class it does not.
    #[must_use]
    pub fn groups(&self) -> Option<&ConditionGroups> {
        self.groups.as_deref()
    }
}
