// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The verdicts a query's access condition holds.
//!
//! Each settings class carries a pair of verdicts, with care and without,
//! and the facts of a class are read against the caller's keys. No openEHR
//! spec governs this: our own design (#5513, #5588).

use std::sync::Arc;

use uuid::Uuid;

use crate::access_eval::procedure::Verdict;

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
