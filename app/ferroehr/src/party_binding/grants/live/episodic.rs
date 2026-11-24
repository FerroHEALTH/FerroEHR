// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The carers a care relationship scoped to an episode targets, held beside
//! the caller facts and moved with them, so a request knows from the copy it
//! is judged on whether any of its principals may hold such a relationship.
//! No openEHR spec governs this: our own design (#6414, #6415).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use uuid::Uuid;

use crate::party_binding::grants::caller::store::FactRow;
use crate::party_binding::grants::caller::{FactKind, FactValue};

/// Each subject holding a relationship scoped to an episode, with the
/// carers those relationships target, and each such carer with its subjects.
#[derive(Debug, Default, Clone)]
pub(super) struct Episodic {
    by_subject: BTreeMap<Uuid, Arc<[Uuid]>>,
    by_carer: BTreeMap<Uuid, BTreeSet<Uuid>>,
}

impl Episodic {
    /// These carers once each of `stamped` holds the relationships
    /// `cared` reads for it now, none for a stamped subject it leaves out.
    pub(super) fn with<'a>(
        &self,
        stamped: impl IntoIterator<Item = Uuid>,
        cared: impl IntoIterator<Item = (Uuid, &'a [FactRow])>,
    ) -> Self {
        let mut next = self.clone();
        for subject in stamped {
            next.forget(subject);
        }
        for (subject, rows) in cared {
            next.hold(subject, rows);
        }
        next
    }

    /// The carers the relationships scoped to an episode among `parties`
    /// target, those relationships taken out of the parties' facts and a
    /// party left with no fact dropped.
    pub(super) fn split(parties: &mut Vec<(Uuid, Vec<FactRow>)>) -> Self {
        let mut episodic = Self::default();
        for (party, facts) in parties.iter_mut() {
            let (care, kept): (Vec<FactRow>, Vec<FactRow>) = std::mem::take(facts)
                .into_iter()
                .partition(|row| row.kind == FactKind::UnderCareOf);
            episodic.hold(*party, &care);
            *facts = kept;
        }
        parties.retain(|(_, facts)| !facts.is_empty());
        episodic
    }

    /// Whether a relationship scoped to an episode targets any carer.
    pub(super) fn any(&self) -> bool {
        !self.by_carer.is_empty()
    }

    /// Whether a relationship scoped to an episode targets one of `carers`.
    pub(super) fn targets_any(&self, carers: &[Uuid]) -> bool {
        carers.iter().any(|carer| self.by_carer.contains_key(carer))
    }

    /// Holds the carers of the relationships of `subject` among `rows` that
    /// name an episode.
    pub(super) fn hold(&mut self, subject: Uuid, rows: &[FactRow]) {
        let carers: BTreeSet<Uuid> = rows
            .iter()
            .filter(|row| row.kind == FactKind::UnderCareOf)
            .filter_map(|row| match &row.value {
                FactValue::Care(care) if care.episode.is_some() => Some(care.carer),
                _ => None,
            })
            .collect();
        if carers.is_empty() {
            return;
        }
        for carer in &carers {
            self.by_carer.entry(*carer).or_default().insert(subject);
        }
        self.by_subject
            .insert(subject, carers.into_iter().collect());
    }

    fn forget(&mut self, subject: Uuid) {
        let Some(carers) = self.by_subject.remove(&subject) else {
            return;
        };
        for carer in carers.iter() {
            if let Some(subjects) = self.by_carer.get_mut(carer) {
                subjects.remove(&subject);
                if subjects.is_empty() {
                    self.by_carer.remove(carer);
                }
            }
        }
    }
}

impl super::LiveGrants {
    /// Whether the caller facts copy held knows any care relationship scoped
    /// to an episode.
    #[must_use]
    pub fn names_episodes(&self) -> bool {
        self.held_facts().episodic.any()
    }

    /// Whether a care relationship scoped to an episode targets one of
    /// `principals`, live or not, from the caller facts copy held.
    #[must_use]
    pub fn episodic(&self, principals: &[Uuid]) -> bool {
        self.held_facts().episodic.targets_any(principals)
    }
}
