// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use std::time::{Duration, Instant};

use super::Rejudge;
use crate::access_eval::stale::STALE_WAIT;
use crate::aql::error::StaleCopy;

/// A read of the copies slower than the wait, on a store that holds still,
/// answers: each copy caught up once is no sign the store moves, however
/// long the reads took (#6546).
#[test]
fn a_slow_round_on_a_store_that_holds_still_answers() {
    let start = Instant::now();
    let slow = STALE_WAIT + Duration::from_millis(50);
    let mut rejudge = Rejudge::default();
    let moved = rejudge.stale(StaleCopy::Pinned, start);
    assert!(
        !moved,
        "the first stale copy is a copy behind, not a moving store"
    );
    assert!(
        !rejudge.refuses(moved, start + slow),
        "the slow first round answers"
    );
    let moved = rejudge.stale(StaleCopy::Classes, start + slow);
    assert!(!moved, "another copy behind is still no moving store");
    assert!(
        !rejudge.refuses(moved, start + slow + slow),
        "the second copy caught up after a slow round answers"
    );
}

/// A copy the query read again and found older once more is a store that
/// moves: it runs again within the wait and is refused once the wait has
/// passed (#6546).
#[test]
fn a_store_that_moves_on_every_round_is_refused_after_the_wait() {
    let start = Instant::now();
    let mut rejudge = Rejudge::default();
    assert!(!rejudge.stale(StaleCopy::Pinned, start));
    let moved = rejudge.stale(StaleCopy::Pinned, start + Duration::from_millis(10));
    assert!(moved, "the copy read again is older once more");
    assert!(
        !rejudge.refuses(moved, start + Duration::from_millis(20)),
        "within the wait the query runs again"
    );
    assert!(
        rejudge.refuses(moved, start + STALE_WAIT),
        "once the wait has passed the query is refused"
    );
}
