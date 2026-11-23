// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A grant holds within its dates and instants under the ceiling, a change
//! is tracked inside its request only, and queued stale refreshes run one
//! load (#6343).

use super::*;
use crate::party_binding::grants::store::{Dates, Term};

fn date(text: &str) -> jiff::civil::Date {
    text.parse().expect("a date")
}

fn row(code: &str) -> CapabilityRow {
    CapabilityRow::new(
        Uuid::nil(),
        Uuid::nil(),
        code.to_owned(),
        Term {
            lifecycle_state: "532".to_owned(),
            role_dates: Some(Dates {
                lower: None,
                upper: None,
            }),
            dates: Some(Dates {
                lower: Some(date("2026-12-01")),
                upper: Some(date("2026-12-03")),
            }),
            not_before: None,
            not_after: None,
        },
        None,
    )
}

#[test]
fn a_grant_holds_within_its_dates_and_instants_under_the_ceiling() {
    let ceiling = GrantCeiling::new(Operation::ALL);
    let now: jiff::Timestamp = "2026-12-02T12:00:00Z".parse().expect("an instant");
    let today = date("2026-12-02");
    let relay = row("commit_composition");
    assert_eq!(
        in_force(&relay, &ceiling, today, now),
        Some(Operation::CommitComposition)
    );
    assert_eq!(
        in_force(&relay, &ceiling, date("2026-12-03"), now),
        None,
        "the upper bound is exclusive"
    );
    assert_eq!(
        in_force(&relay, &GrantCeiling::new([]), today, now),
        None,
        "an operation the ceiling does not admit is never honoured"
    );
    let mut timed = relay.clone();
    timed.term.not_before = Some("2026-12-02T13:00:00Z".parse().expect("an instant"));
    assert_eq!(in_force(&timed, &ceiling, today, now), None);
    timed.term.not_before = None;
    timed.term.not_after = Some("2026-12-02T11:00:00Z".parse().expect("an instant"));
    assert_eq!(in_force(&timed, &ceiling, today, now), None);
    let mut empty = relay.clone();
    empty.term.role_dates = None;
    assert_eq!(in_force(&empty, &ceiling, today, now), None);
    let mut inactive = relay;
    inactive.term.lifecycle_state = "800".to_owned();
    assert_eq!(in_force(&inactive, &ceiling, today, now), None);
    assert_eq!(
        in_force(&row("import_everything"), &ceiling, today, now),
        None
    );
}

/// Refreshes that all found the copy stale and queued on `installing` run
/// one load between them: each after the first finds the revision it
/// read installed (#6343).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_stale_refreshes_make_one_load() {
    let db = testkit::db().await.expect("testkit database");
    let pool = crate::db::domain_pool_from(&db.pool(), Domain::Party);
    let live = Arc::new(LiveGrants::new(pool));
    let gate = live.installing.lock().await;
    let refreshes: Vec<_> = (0..6)
        .map(|_| {
            let live = Arc::clone(&live);
            tokio::spawn(async move { live.refresh().await })
        })
        .collect();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    drop(gate);
    for refresh in refreshes {
        refresh
            .await
            .expect("the refresh ends")
            .expect("the store answers");
    }
    assert_eq!(live.loads.load(Ordering::Acquire), 1, "one load");
    assert!(live.revision().is_some(), "the revision is installed");
}

#[tokio::test]
async fn a_change_is_tracked_inside_the_request_only() {
    let ((), changed) = tracking(async { note_change() }).await;
    assert!(changed);
    let ((), changed) = tracking(async {}).await;
    assert!(!changed);
    note_change();
}
