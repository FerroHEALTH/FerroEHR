// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The condition a query applies to every EHR at once: the settings classes
//! grouped by the verdicts this request judges them to, built once per query.

use std::collections::BTreeMap;
use std::sync::Arc;

use uuid::Uuid;

use super::care::Care;
use super::records::Held;
use super::{AccessCondition, CallerKeys, ConditionGroups, RequestAccess, VerdictPair, pinned};
use crate::access_eval::class::Classes;
use crate::service::ehr::access_settings::EhrAccessSettings;

impl RequestAccess {
    /// The condition a query applies, over every EHR of a class among
    /// `classes`, an EHR held closed, and the default for an EHR with no
    /// settings, the caller's care read as `care` says. An EHR of a class
    /// this replica does not hold is withheld.
    #[must_use]
    pub fn condition(&self, classes: &Classes, care: Care) -> AccessCondition {
        let held = Held::of_class(self.confined());
        let pair = |settings: Option<&EhrAccessSettings>| VerdictPair {
            cared: self.judge_as(held, settings, true).bare(),
            uncared: self.judge_as(held, settings, false).bare(),
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
            care,
            self.grants
                .as_ref()
                .map_or_else(|| Arc::from(""), |grants| grants.namespace()),
            facts,
            // NOTE: our own design (#5586), a confined caller reads only the EHR its patient
            // context names, so a query of it reaches no other EHR whatever its class.
            held.own
                .then(|| self.caller.own_ehrs.iter().map(|own| own.0).collect()),
        )
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

    /// The condition of the classes `groups`, `closed` for an EHR held
    /// closed and `default` for an EHR with no settings.
    fn new(
        groups: BTreeMap<VerdictPair, Vec<i64>>,
        closed: VerdictPair,
        default: VerdictPair,
        cared: Care,
        subject_namespace: Arc<str>,
        facts: CallerKeys,
        confined: Option<Arc<[Uuid]>>,
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
            cared,
            subject_namespace,
            facts,
            confined,
        }
        .into()
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
