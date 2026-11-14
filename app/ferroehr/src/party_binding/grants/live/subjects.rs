// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What [`LiveGrants`] answers about the subject of an EHR from its copy.
//!
//! It answers with no read of its own: the `PERSON` an EHR subject reference
//! names, the references that name a `PERSON`, and the care relationships
//! that hang off it. No openEHR spec governs where the subject's party is
//! found: our own design (#5634).

use uuid::Uuid;

use super::LiveGrants;
use crate::party_binding::grants::caller::{CareTeam, FactKind, FactValue};
use crate::storage::access_projection::EhrSubject;

impl LiveGrants {
    /// The care teams and units the subject `subject` is under the care of,
    /// from the copy held: each `under_care_of` relationship of its live
    /// `PERSON` head with the dates it holds, whether or not it holds today,
    /// so the evaluator judges a discharge and the grace after it.
    #[must_use]
    pub fn under_care_of(&self, subject: Uuid) -> Vec<CareTeam> {
        let held = self.held_facts();
        let mut out: Vec<CareTeam> = held
            .by_party
            .get(&subject)
            .into_iter()
            .flat_map(|rows| rows.iter())
            .filter(|row| row.kind == FactKind::UnderCareOf)
            .filter(|row| matches!(row.term.lifecycle_state.as_str(), "532" | "553"))
            .filter_map(|row| match (row.value.clone(), row.term.dates) {
                (FactValue::Team(team), Some(dates)) => Some(CareTeam::new(team, dates)),
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// The `PERSON` the EHR subject reference `subject` names, from the copy
    /// held, with no read of its own.
    ///
    /// A reference in this CDR's own namespace whose id is a UUID names the
    /// `PERSON` of that container; any other names the `complete` or
    /// `incomplete` `PERSON` that declares it by a record subject element;
    /// otherwise there is none, and a step reading the subject's `PERSON`
    /// fails closed. Both are legal subject references (BASE `OBJECT_REF`,
    /// RM Common IM §`PARTY_SELF`), so the namespace is compared with the id.
    /// The order is our own design (#5634).
    #[must_use]
    pub fn person_of(&self, subject: &EhrSubject) -> Option<Uuid> {
        if *subject.namespace == *self.namespace() {
            // NOTE: the local source names each party by its container's UUID, so an id
            // in the own namespace that is no UUID names no PERSON of this store.
            return subject.id.parse::<Uuid>().ok();
        }
        let held = self.held_facts();
        let person = *held.person_by_subject.get(subject)?;
        held.subjects_of
            .get(&person)?
            .iter()
            .any(|row| row.subject == *subject && row.live())
            .then_some(person)
    }

    /// The subject references that name the `PERSON` `person`, from the copy
    /// held: its own reference in this CDR's namespace, then each record
    /// subject it declares while it is `complete` or `incomplete`.
    #[must_use]
    pub fn subjects_of(&self, person: Uuid) -> Vec<EhrSubject> {
        let mut out = vec![EhrSubject::new(&*self.namespace(), person.to_string())];
        let held = self.held_facts();
        out.extend(
            held.subjects_of
                .get(&person)
                .into_iter()
                .flat_map(|rows| rows.iter())
                .filter(|row| row.live())
                .map(|row| row.subject.clone()),
        );
        out
    }

    /// The subjects under the care of any of `teams` on `day`, with `grace`
    /// after each relationship ends, from the copy held: the `PERSON`s a
    /// member of those teams has a live care relationship to
    /// ([`CareTeam::live_on`]), each with the subject references that name
    /// it ([`Self::subjects_of`]).
    #[must_use]
    pub fn cared_for_by(
        &self,
        teams: &[Uuid],
        day: jiff::civil::Date,
        grace: jiff::Span,
    ) -> Vec<EhrSubject> {
        if teams.is_empty() {
            return Vec::new();
        }
        let held = self.held_facts();
        let persons: Vec<Uuid> = held
            .by_party
            .iter()
            .filter(|(_, rows)| {
                rows.iter().any(|row| {
                    row.kind == FactKind::UnderCareOf
                        && matches!(row.term.lifecycle_state.as_str(), "532" | "553")
                        && match (&row.value, row.term.dates) {
                            (FactValue::Team(team), Some(dates)) => {
                                teams.contains(team)
                                    && CareTeam::new(*team, dates).live_on(day, grace)
                            }
                            _ => false,
                        }
                })
            })
            .map(|(subject, _)| *subject)
            .collect();
        drop(held);
        persons
            .into_iter()
            .flat_map(|person| self.subjects_of(person))
            .collect()
    }
}
