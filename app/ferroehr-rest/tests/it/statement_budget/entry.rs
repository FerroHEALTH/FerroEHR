// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The allocations one access-log entry costs the request that writes it
//! (#6005): the entry built from the access's audit event, its BALP
//! projection and search tokens, its stored parts, digests and derived
//! columns, and the intake write handed to the gathered writer, counted on
//! the thread that polls the write. No openEHR spec governs the cost of a
//! record: our own design.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use ferroehr::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use ferroehr::system_log::store::{AuditRecord, AuditStore};

/// The entries measured, after as many written first.
const ENTRIES: u64 = 50;

/// The most allocations one entry may cost, measured when the search tokens
/// came from the typed BALP resource and the parts were cut without a JSON
/// value (#6005), 2% above. Before, an entry cost 685. The search tokens
/// have since been rendered by the frontier round from the stored entry
/// (#6322): an entry measured 145 against 279 beside the merge base, and the
/// ceiling fell by the drop.
const CEILING: u64 = 152;

/// A future whose every poll is counted into `count`.
struct Counted<'a, F> {
    inner: Pin<Box<F>>,
    count: &'a mut u64,
}

impl<F: Future> Future for Counted<'_, F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let this = &mut *self;
        let mut out = Poll::Pending;
        let info = allocation_counter::measure(|| out = this.inner.as_mut().poll(cx));
        *this.count += info.count_total;
        out
    }
}

/// The audit record of a composition read by a nurse, the `n`th.
fn read(n: u64) -> AuditRecord {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "nurse".clone_into(&mut event.user_id);
    event.domain = AccessDomain::Ehr;
    event.ehr_id = Some(format!("7d44b88c-4199-4bad-97dc-{n:012x}"));
    event.object_id = Some(format!(
        "8849182c-82ad-4088-a07f-{n:012x}::cdr.example.org::1"
    ));
    event.request_id = Some(format!("d44e8723-cf7e-4b37-8f52-{n:012x}"));
    event.event_type = Some(EventType::rest("composition_get"));
    event.roles = vec!["USER".to_owned()];
    event.client_ip = Some("10.0.0.7".to_owned());
    AuditRecord {
        event,
        subject: Some(format!("patient-{n}")),
        entry: None,
    }
}

/// One access-log entry costs the request that writes it at most
/// [`CEILING`] allocations, gated on Linux, where the ceiling is measured
/// (`super::paths::ALLOCATIONS_GATED`); elsewhere the figure is reported.
#[tokio::test]
async fn an_access_log_entry_stays_within_its_allocation_budget() {
    let db = testkit::db().await.expect("testkit database");
    let store = AuditStore::new(db.pool());
    for n in 0..ENTRIES {
        store
            .write(vec![read(n)], &[])
            .await
            .expect("the entry is written");
    }
    let mut count = 0;
    for n in ENTRIES..ENTRIES * 2 {
        Counted {
            inner: Box::pin(store.write(vec![read(n)], &[])),
            count: &mut count,
        }
        .await
        .expect("the entry is written");
    }
    let per_entry = count.div_euclid(ENTRIES);
    eprintln!("an access-log entry: {per_entry} allocations (ceiling {CEILING})");
    assert!(
        !super::paths::ALLOCATIONS_GATED || per_entry <= CEILING,
        "an access-log entry costs {per_entry} allocations, above its ceiling of {CEILING}"
    );
}
