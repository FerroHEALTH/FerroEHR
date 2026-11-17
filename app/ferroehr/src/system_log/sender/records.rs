// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The records of one act as the store writes them: each event with the
//! subject of its EHR and the access-log entry of an access to EHR data
//! (#5127). No openEHR spec governs the audit trail: our own design.

use std::collections::BTreeSet;

use super::SubjectCache;
#[cfg(feature = "fhir")]
use crate::access_log::entry::for_store;
use crate::system_log::event::AuditEvent;
use crate::system_log::message::AuditContext;
use crate::system_log::store::AuditRecord;

/// The records of `events`, each with its memoized subject (or its lookup
/// marked failed when its EHR is in `unavailable`) and the access-log entry
/// it writes. `unavailable` is `None` when subjects are not resolved at all.
///
/// Every event time is cut to the microsecond the store keeps, so a document
/// rendered from the record and one rendered from the store name the same
/// instant.
pub(super) async fn render_records(
    events: Vec<AuditEvent>,
    ctx: &AuditContext,
    subject_cache: &SubjectCache,
    unavailable: Option<&BTreeSet<String>>,
) -> Vec<AuditRecord> {
    #[cfg(not(feature = "fhir"))]
    let _: &AuditContext = ctx;
    let mut records: Vec<AuditRecord> = Vec::with_capacity(events.len());
    for mut event in events {
        event.timestamp = crate::system_log::event::stored_instant(event.timestamp);
        let resolved = match (unavailable, &event.ehr_id) {
            (Some(failed), Some(ehr_id)) if failed.contains(ehr_id) => {
                event.subject_lookup_failed = true;
                None
            }
            (Some(_), Some(ehr_id)) => subject_cache.get(ehr_id).await.flatten(),
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
