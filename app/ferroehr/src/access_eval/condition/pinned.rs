// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The condition of a query whose rows all belong to one EHR.
//!
//! The evaluator decides that EHR once, from this replica's copy of its
//! settings and subject, as a direct read of it is decided
//! ([`RequestAccess::judge`]), so the statement carries only what the verdict
//! leaves to each object (its template, root archetype, origins, commit time
//! and id) instead of the class condition with its probe of the EHR's
//! settings row for every object (#5746). The statement checks once that the
//! revision the verdict was judged on is the one in force, so a verdict
//! judged on a copy older than the store fails the statement and is judged
//! again (`crate::service::query`, #5794). No openEHR spec
//! governs this: our own design.

use std::sync::Arc;

use uuid::Uuid;

use super::care::Care;
use super::{AccessCondition, EhrEntry, RequestAccess};
use crate::access_eval::care_tie::Cared;
use crate::access_eval::scope::Pred;
use crate::ids::EhrId;
use crate::storage::access_projection::Revision;

/// The verdict of the one EHR a pinned query reads, as its statement applies
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedEhr {
    /// The EHR.
    pub ehr: Uuid,
    /// The objects of the EHR the caller may read, over the facts of each
    /// object alone; [`Pred::False`] when the caller may read none of it.
    pub admitted: Pred,
    /// Whether the EHR has a settings row.
    pub settled: bool,
    /// The revision of the EHR's settings row and subject the verdict was
    /// judged on.
    pub revision: Revision,
}

impl PinnedEhr {
    /// The revision as the store's `access_revision_of` view writes it: the JSON
    /// array `[<ordinal>, <namespace>, <id>]`, each null when absent.
    #[must_use]
    pub fn revision_json(&self) -> String {
        let subject = self.revision.subject.as_ref();
        serde_json::json!([
            self.revision.settings,
            subject.map(|s| s.namespace.as_str()),
            subject.map(|s| s.id.as_str()),
        ])
        .to_string()
    }
}

impl RequestAccess {
    /// `condition` for a query whose rows all belong to `ehr_id`, decided
    /// from `entry`, its copy, as a read of that EHR is decided.
    ///
    /// The caller's care for its subject is the one `condition` already
    /// reads: the EHRs it found cared for, or none when no verdict reads
    /// care. A condition whose care is read in another form, or a verdict
    /// that leaves a fact of the EHR to the statement, keeps `condition` as
    /// it is, as does a condition already pinned to `ehr_id`.
    #[must_use]
    pub fn pin(
        &self,
        condition: &AccessCondition,
        ehr_id: EhrId,
        entry: &EhrEntry,
    ) -> AccessCondition {
        let Some(groups) = condition.groups() else {
            return condition.clone();
        };
        if condition
            .pinned()
            .is_some_and(|pinned| pinned.ehr == ehr_id.0)
        {
            return condition.clone();
        }
        let care = match &groups.cared {
            Care::Ehrs(ehrs) => ehrs
                .iter()
                .find(|cared| cared.ehr == ehr_id.0)
                .map_or(Cared::No, |cared| Cared::Ties(Arc::clone(&cared.ties))),
            _ if !condition.reads_care() => Cared::No,
            _ => return condition.clone(),
        };
        let verdict = self.judge(Some(ehr_id), (*entry.settings).as_ref(), &care);
        let admitted = if verdict.admits_any() {
            verdict.admitted()
        } else {
            Pred::False
        };
        if reads_ehr_facts(&admitted) {
            return condition.clone();
        }
        AccessCondition {
            groups: condition.groups.clone(),
            pinned: Some(Arc::new(PinnedEhr {
                ehr: ehr_id.0,
                admitted,
                settled: entry.revision.settings.is_some(),
                revision: entry.revision.clone(),
            })),
        }
    }
}

impl AccessCondition {
    /// The verdict of the one EHR the query reads, when the service decided
    /// it before the statement ([`RequestAccess::pin`]).
    #[must_use]
    pub fn pinned(&self) -> Option<&PinnedEhr> {
        self.pinned.as_deref()
    }
}

/// Whether `pred` reads a fact of the EHR, which only a settings class
/// leaves to the statement.
fn reads_ehr_facts(pred: &Pred) -> bool {
    match pred {
        Pred::Ehr(_) | Pred::Care(_) => true,
        Pred::True | Pred::False | Pred::Data(..) => false,
        Pred::Not(inner) => reads_ehr_facts(inner),
        Pred::All(preds) | Pred::Any(preds) => preds.iter().any(reads_ehr_facts),
    }
}
