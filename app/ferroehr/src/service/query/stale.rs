// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A query judged on an access copy older than the store.
//!
//! The access condition of a query is judged from this replica's copies of
//! the settings classes and, for a query pinned to one EHR, of that EHR's
//! settings and subject. The statement fails when a copy is older than the
//! store (`clinical/0049_access_stale_copy.sql`), so a stale verdict never
//! answers rows, a count or a page, and never an empty result that reads as
//! "nothing recorded". The service then drops the stale copy, judges the
//! query again from a fresh read, as a direct read judged on an older
//! revision is judged again (`FerroEhrService::ehr_verdict_at`), and runs
//! it once more; while the store keeps moving it tries again for at most
//! [`STALE_WAIT`], and then refuses with `503` and `Retry-After`
//! ([`ServiceError::access_refreshing`]). Each case is counted on
//! `aql_stale_access_total{copy, outcome}`. No openEHR spec governs the
//! copies or the refusal: our own design (#5794).

use std::time::{Duration, Instant};

use crate::access_eval::condition::{AccessCondition, current};
use crate::aql::error::StaleCopy;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::FerroEhrService;

/// How long a query asks again after its first stale statement.
pub(super) const STALE_WAIT: Duration = Duration::from_millis(100);

/// The pause between two judgements that found the copy unchanged.
const STALE_POLL: Duration = Duration::from_millis(10);

impl FerroEhrService {
    /// The condition of a query whose statement found `copy` stale, judged
    /// again from a fresh read: `base` (the request's unresolved condition
    /// outside a request) with its care resolved and its EHR decided for
    /// `pinned`, as [`Self::query_access_condition`] does.
    ///
    /// A judgement equal to `stale` reads the copy again after a pause; once
    /// [`STALE_WAIT`] has passed since `since`, the first stale statement,
    /// the query is refused.
    ///
    /// # Errors
    /// [`ServiceError::AccessRefreshing`] (`503`) once the wait has passed,
    /// and the errors of the reads.
    pub(super) async fn judged_again(
        &self,
        copy: StaleCopy,
        base: &AccessCondition,
        pinned: Option<&[EhrId]>,
        stale: &AccessCondition,
        since: Instant,
    ) -> Result<AccessCondition, ServiceError> {
        loop {
            if since.elapsed() >= STALE_WAIT {
                count(copy, "refused");
                return Err(ServiceError::access_refreshing());
            }
            match copy {
                StaleCopy::Pinned => {
                    for ehr_id in pinned.unwrap_or_default() {
                        self.ehr_access.forget(*ehr_id).await;
                    }
                }
                StaleCopy::Classes => self.ehr_access.forget_classes(),
            }
            let unresolved = if current().is_some() {
                self.request_access_condition().await?
            } else {
                base.clone()
            };
            let fresh = Box::pin(self.query_access_condition(&unresolved, pinned)).await?;
            if fresh != *stale {
                count(copy, "judged_again");
                return Ok(fresh);
            }
            tokio::time::sleep(STALE_POLL).await;
        }
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
