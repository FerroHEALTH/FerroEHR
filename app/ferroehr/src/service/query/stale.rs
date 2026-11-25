// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A query judged on an access copy older than the store.
//!
//! The access condition of a query is judged from this replica's copies of
//! the settings classes and, for a query pinned to one EHR, of that EHR's
//! settings and subject. The statement fails when a copy is older than the
//! store (`clinical/0009_baseline_access.sql`), so a stale verdict never
//! answers rows, a count or a page, and never an empty result that reads as
//! "nothing recorded". The service then drops the stale copy, judges the
//! query again from a fresh read, as a direct read judged on an older
//! revision is judged again (`FerroEhrService::ehr_verdict_at`), and runs
//! it once more. It refuses with `503` and `Retry-After`
//! ([`ServiceError::access_refreshing`]) only while the store keeps moving:
//! once a copy it already read again is found older once more and
//! [`STALE_WAIT`] has passed since the first stale statement, or once a
//! fresh read stays equal to the stale one for [`STALE_WAIT`] of pauses.
//! Each case is counted on `aql_stale_access_total{copy, outcome}`. No
//! openEHR spec governs the copies or the refusal: our own design (#5794).

use std::borrow::Cow;
use std::time::{Duration, Instant};

use crate::access_eval::condition::{AccessCondition, current};
use crate::access_eval::stale::{STALE_POLL, STALE_WAIT};
use crate::aql::error::StaleCopy;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::query::request::QueryAccess;

/// What a query's re-judge loop has seen of the store since its first
/// statement that found a copy stale.
#[derive(Debug, Default)]
pub(super) struct Rejudge {
    /// When the first statement found a copy stale.
    since: Option<Instant>,
    /// The copies this query has read again.
    read_again: Vec<StaleCopy>,
}

impl Rejudge {
    /// Records a statement that found `copy` stale at `at`, and answers
    /// whether the store moved past this query's own fresh read of `copy`.
    fn stale(&mut self, copy: StaleCopy, at: Instant) -> bool {
        self.since.get_or_insert(at);
        if self.read_again.contains(&copy) {
            return true;
        }
        self.read_again.push(copy);
        false
    }

    /// Whether a fresh judgement read at `at` refuses the query: only when
    /// the store moved and [`STALE_WAIT`] has passed since the first stale
    /// statement, so a slow read of a store that holds still never does.
    fn refuses(&self, moved: bool, at: Instant) -> bool {
        moved
            && self
                .since
                .is_some_and(|since| at.saturating_duration_since(since) >= STALE_WAIT)
    }
}

impl FerroEhrService {
    /// The condition of a query whose statement found `copy` stale, judged
    /// again from a fresh read: the running request's condition, or the one
    /// `base` gives outside a request, with its care resolved and its EHR decided for
    /// `pinned`, as [`Self::query_access_condition`] does.
    ///
    /// A judgement equal to `stale` reads the copy again after a pause, for
    /// at most [`STALE_WAIT`] of pauses; a judgement that differs refuses
    /// the query when [`Rejudge::refuses`] says the store kept moving.
    ///
    /// # Errors
    /// [`ServiceError::AccessRefreshing`] (`503`) when the store kept moving
    /// or the copy did not catch up, and the errors of the reads.
    pub(super) async fn judged_again(
        &self,
        copy: StaleCopy,
        base: &QueryAccess,
        pinned: Option<&[EhrId]>,
        stale: &AccessCondition,
        rejudge: &mut Rejudge,
    ) -> Result<AccessCondition, ServiceError> {
        let moved = rejudge.stale(copy, Instant::now());
        let mut paused = Duration::ZERO;
        loop {
            match copy {
                StaleCopy::Pinned => {
                    for ehr_id in pinned.unwrap_or_default() {
                        self.ehr_access.forget(*ehr_id).await;
                    }
                }
                StaleCopy::Classes => self.ehr_access.forget_classes(),
            }
            let unresolved = match base {
                QueryAccess::Given(given) if current().is_none() => Cow::Borrowed(given),
                _ => Cow::Owned(Box::pin(self.request_access_condition()).await?),
            };
            let fresh = Box::pin(self.query_access_condition(&unresolved, pinned)).await?;
            if fresh != *stale {
                if rejudge.refuses(moved, Instant::now()) {
                    break;
                }
                count(copy, "judged_again");
                return Ok(fresh);
            }
            if paused >= STALE_WAIT {
                break;
            }
            tokio::time::sleep(STALE_POLL).await;
            paused = paused.saturating_add(STALE_POLL);
        }
        count(copy, "refused");
        Err(ServiceError::access_refreshing())
    }
}

/// Increment `aql_stale_access_total{copy, outcome}`.
fn count(copy: StaleCopy, outcome: &'static str) {
    crate::telemetry::metrics::metrics().aql_stale_access.add(
        1,
        &[
            opentelemetry::KeyValue::new("copy", copy.label()),
            opentelemetry::KeyValue::new("outcome", outcome),
        ],
    );
}

#[cfg(test)]
#[path = "stale_tests.rs"]
mod tests;
