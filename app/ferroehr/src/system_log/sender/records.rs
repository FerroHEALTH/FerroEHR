// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The records of one act as the store writes them: each event with the
//! subject of its EHR and the access-log entry of an access to EHR data
//! (#5127). No openEHR spec governs the audit trail: our own design.

use std::collections::{BTreeMap, BTreeSet};

use super::ResolvedSubject;
#[cfg(feature = "fhir")]
use crate::access_log::entry::for_store;
use crate::system_log::event::AuditEvent;
use crate::system_log::message::AuditContext;
use crate::system_log::store::AuditRecord;

/// The subjects of the EHRs one act's records reference, as the act read
/// them: each EHR's subject or its absence, and the EHRs whose lookup
/// failed. The records are rendered from these, never from the memo again,
/// so an entry the memo's cap evicts meanwhile changes no record (#5926).
#[derive(Debug, Default)]
pub(super) struct Subjects {
    /// Each EHR's subject, `None` for one that names none.
    pub(super) resolved: BTreeMap<String, Option<ResolvedSubject>>,
    /// The EHRs whose lookup failed.
    pub(super) unavailable: BTreeSet<String>,
}

/// The records of `events`, each with its subject from `subjects` (or its
/// lookup marked failed when its EHR's lookup failed) and the access-log
/// entry it writes. `subjects` is `None` when subjects are not resolved at
/// all.
///
/// Every event time is cut to the microsecond the store keeps, so a document
/// rendered from the record and one rendered from the store name the same
/// instant.
pub(super) fn render_records(
    events: Vec<AuditEvent>,
    ctx: &AuditContext,
    subjects: Option<&Subjects>,
) -> Vec<AuditRecord> {
    #[cfg(not(feature = "fhir"))]
    let _: &AuditContext = ctx;
    let mut records: Vec<AuditRecord> = Vec::with_capacity(events.len());
    for mut event in events {
        event.timestamp = crate::system_log::event::stored_instant(event.timestamp);
        let resolved = match (subjects, &event.ehr_id) {
            (Some(subjects), Some(ehr_id)) if subjects.unavailable.contains(ehr_id) => {
                event.subject_lookup_failed = true;
                None
            }
            (Some(subjects), Some(ehr_id)) => subjects.resolved.get(ehr_id).cloned().flatten(),
            _ => None,
        };
        let (subject, namespace) = match resolved {
            Some(resolved) => (Some(resolved.id), resolved.namespace),
            None => (None, None),
        };
        event.subject_namespace = namespace;
        #[cfg(feature = "fhir")]
        let entry = for_store(&event, subject.as_deref(), ctx);
        #[cfg(not(feature = "fhir"))]
        let entry = None;
        records.push(AuditRecord {
            event,
            subject,
            entry,
        });
    }
    records
}
