// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The drain's records and what each sink receives of them: the store the
//! record, the syslog sink its DICOM audit message and the store-off FHIR
//! feed its `AuditEvent`, each a projection of the record's access-log entry
//! when it writes one (#5127). No openEHR spec governs the audit trail: our
//! own design.

#![cfg_attr(
    feature = "fhir",
    expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the drain carries an \
              already-rendered FHIR document, never a typed resource"
    )
)]

use std::collections::BTreeSet;

#[cfg(feature = "fhir")]
use ihe_atna::atna::forwarder::Forwarder;
#[cfg(feature = "fhir")]
use ihe_atna::atna::spool::Spool;
use secrecy::SecretSlice;

use super::categories::Classifiers;
use super::{Drained, Sinks, SubjectCache};
#[cfg(feature = "fhir")]
use crate::access_log::entry::for_store;
#[cfg(feature = "fhir")]
use crate::access_log::projection::Projection;
use crate::system_log::AuditError;
use crate::system_log::event::AuditEvent;
use crate::system_log::message::AuditContext;
use crate::system_log::store::AuditRecord;
use crate::system_log::syslog::SyslogRoute;

/// Drains the batch into records, attaching each event's memoized subject (or
/// marking its lookup failed when its EHR is in `unavailable`), the
/// access-log entry it writes and, when the store-off FHIR feed is on, its
/// rendered `AuditEvent` document. `unavailable` is `None` when subjects are
/// not resolved at all.
///
/// The store renders its documents when they are read, so with the store on
/// nothing is rendered here. Every event time is cut to the microsecond the
/// store keeps, so a document rendered now and one rendered from the store
/// name the same instant.
pub(super) async fn render_batch(
    batch: &mut Vec<AuditEvent>,
    ctx: &AuditContext,
    classifiers: &Classifiers,
    subject_cache: &SubjectCache,
    unavailable: Option<&BTreeSet<String>>,
    sinks: &Sinks,
) -> Drained {
    #[cfg(not(feature = "fhir"))]
    let _: (&Sinks, &AuditContext, &Classifiers) = (sinks, ctx, classifiers);
    let mut records: Vec<AuditRecord> = Vec::with_capacity(batch.len());
    #[cfg(feature = "fhir")]
    let mut rendered: Vec<Option<serde_json::Value>> = Vec::with_capacity(batch.len());
    for event in batch.drain(..) {
        let mut event = event;
        event.timestamp = stored_instant(event.timestamp);
        let resolved = match (unavailable, &event.ehr_id) {
            (Some(failed), Some(ehr_id)) if failed.contains(ehr_id) => {
                event.subject_lookup_failed = true;
                None
            }
            (Some(_), Some(ehr_id)) => subject_cache.get(ehr_id).await.flatten(),
            _ => None,
        };
        event
            .subject_namespace
            .clone_from(&resolved.as_ref().and_then(|s| s.namespace.clone()));
        let subject = resolved.map(|s| s.id);
        #[cfg(feature = "fhir")]
        let entry = for_store(&event, subject.as_deref(), ctx);
        #[cfg(not(feature = "fhir"))]
        let entry = None;
        let record = AuditRecord {
            event,
            subject,
            entry,
        };
        #[cfg(feature = "fhir")]
        rendered.push(
            sinks
                .feed_spool
                .is_some()
                .then(|| feed_document(&record, ctx, classifiers))
                .flatten(),
        );
        records.push(record);
    }
    Drained {
        records,
        #[cfg(feature = "fhir")]
        rendered,
    }
}

/// The FHIR `AuditEvent` the store-off feed sends of `record`: the projection
/// of its entry, else its own rendering; `None` when it cannot be written
/// (metered and logged, never silent).
#[cfg(feature = "fhir")]
fn feed_document(
    record: &AuditRecord,
    ctx: &AuditContext,
    classifiers: &Classifiers,
) -> Option<serde_json::Value> {
    let Some(entry) = &record.entry else {
        return crate::system_log::fhir::render_audit_event(
            &record.event,
            ctx,
            record.subject.as_deref(),
        );
    };
    let categories = classifiers.of(record.event.category_map_digest);
    let projection = Projection {
        context: ctx,
        categories: &categories,
    };
    crate::system_log::fhir::metered(projection.audit_event(entry, entry.link.sequence))
}

/// `at` cut to the microsecond `timestamptz` keeps; `at` itself when it is
/// already that fine.
fn stored_instant(at: jiff::Timestamp) -> jiff::Timestamp {
    at.round(
        jiff::TimestampRound::new()
            .smallest(jiff::Unit::Microsecond)
            .mode(jiff::RoundMode::Trunc),
    )
    .unwrap_or(at)
}

/// The DICOM PS3.15 A.5 message of `record` as UTF-8 XML: the projection of
/// its entry, else its own rendering.
///
/// # Errors
/// The rendering errors of the message.
fn audit_message(
    record: &AuditRecord,
    ctx: &AuditContext,
    classifiers: &Classifiers,
) -> Result<SecretSlice<u8>, AuditError> {
    #[cfg(feature = "fhir")]
    if let Some(entry) = &record.entry {
        let categories = classifiers.of(record.event.category_map_digest);
        let projection = Projection {
            context: ctx,
            categories: &categories,
        };
        return projection.audit_message(entry, entry.link.sequence);
    }
    #[cfg(not(feature = "fhir"))]
    let _: &Classifiers = classifiers;
    crate::system_log::message::to_xml(&record.event, ctx, record.subject.as_deref())
}

/// Stores every record's DICOM PS3.15 §A.5 XML frame in the syslog spool,
/// which the forwarder delivers.
pub(super) async fn submit_to_syslog(
    route: &SyslogRoute,
    records: &[AuditRecord],
    ctx: &AuditContext,
    classifiers: &Classifiers,
) {
    for record in records {
        let xml = match audit_message(record, ctx, classifiers) {
            Ok(xml) => xml,
            Err(e) => {
                crate::telemetry::metrics::metrics()
                    .atna_audit_serialize_failed
                    .add(1, &[]);
                tracing::warn!("ATNA audit message serialization failed: {e}");
                continue;
            }
        };
        if let Err(e) = route.submit(record.event.timestamp, &xml).await {
            crate::telemetry::metrics::metrics()
                .atna_audit_dropped
                .add(1, &[opentelemetry::KeyValue::new("sink", "syslog")]);
            tracing::warn!(
                "ATNA audit syslog spool refused a record, which syslog never receives: {e}"
            );
        }
    }
}

/// Stores every rendered document in the store-off FHIR feed's spool, which
/// the forwarder delivers.
#[cfg(feature = "fhir")]
pub(super) async fn submit_to_feed_spool(
    forwarder: &Forwarder<Spool>,
    rendered: &[Option<serde_json::Value>],
) {
    for rendered in rendered {
        let Some(document) = rendered else {
            continue;
        };
        let bytes = match serde_json::to_vec(document) {
            Ok(bytes) => bytes,
            Err(e) => {
                crate::telemetry::metrics::metrics()
                    .atna_audit_serialize_failed
                    .add(1, &[]);
                tracing::warn!("ATNA audit FHIR document serialization failed: {e}");
                continue;
            }
        };
        if let Err(e) = forwarder.submit(SecretSlice::from(bytes)).await {
            crate::telemetry::metrics::metrics()
                .atna_audit_dropped
                .add(1, &[opentelemetry::KeyValue::new("sink", "fhir_feed")]);
            tracing::warn!("ATNA audit FHIR feed spool refused a record: {e}");
        }
    }
}
