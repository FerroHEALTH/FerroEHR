// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Admission to the shared test server: a test process waits for room
//! instead of being refused a connection (#6020).
//!
//! Every clone lives on one `PostgreSQL` server, which every test process of
//! every suite on the host shares, and each clone holds a lease connection
//! and a pool. The server refuses a connection past its `max_connections`
//! (`PostgreSQL` 18, "Connections and Authentication",
//! <https://www.postgresql.org/docs/18/runtime-config-connection.html>), and
//! a refused connection failed whichever test asked for it: at 200
//! connections, three suites at once drew 26,762 refusals and failed 255
//! tests that pass alone. So a test process takes one of a fixed number of
//! slots before it creates its first clone and waits while none is free. The
//! number follows the server's own setting, so the processes admitted at
//! once fit the server whatever runs beside them.
//!
//! A slot is a session-level advisory lock on a connection the process keeps
//! until it ends, so the slot is given back however the test ends
//! (`PostgreSQL` 18, "Advisory Locks",
//! <https://www.postgresql.org/docs/18/explicit-locking.html#ADVISORY-LOCKS>).
//! The slot belongs to the process and covers every clone it takes: a slot
//! per clone would stop a suite whose tests each hold one clone and wait for
//! a second. No openEHR spec governs a test harness: our own design.

use std::time::Duration;

use sqlx::PgConnection;
use tokio::sync::{Mutex, OnceCell};

/// The first key of every admission slot; the second is the slot's number.
const SLOT_CLASS: i32 = 0x0EB2_5107;

/// The connections of a clone's pool.
pub(crate) const POOL: u32 = 10;

/// The connections an admitted process is counted as holding: its slot, a
/// lease and a pool's worth. A process seldom fills a pool: 15 admitted at
/// once held nine each at their peak, the tests with two clones among them.
const PER_PROCESS: i64 = 12;

/// How long a process that found no free slot waits before it looks again.
const RETRY: Duration = Duration::from_millis(100);

/// The connection that holds this process's slot, kept until the process
/// ends.
static SLOT: OnceCell<Mutex<PgConnection>> = OnceCell::const_new();

/// The processes a server admits at once: what two thirds of its
/// `max_connections`, less the `reserved` ones, hold of [`PER_PROCESS`], and
/// at least one. The third left over is for the processes that wait, each on
/// one connection, however many suites share the server.
fn slots(max_connections: i64, reserved: i64) -> i32 {
    let room = (max_connections - reserved).saturating_mul(2).div_euclid(3);
    i32::try_from(room.div_euclid(PER_PROCESS).max(1)).unwrap_or(i32::MAX)
}

/// Takes this process's slot of the shared server on a connection `connect`
/// opens and keeps it, waiting while no slot is free; a process that holds
/// its slot opens nothing.
///
/// # Errors
/// [`crate::TestkitError`] from `connect`, and the driver's error when the
/// server stops answering.
pub(crate) async fn admit(
    connect: impl AsyncFnOnce() -> Result<PgConnection, crate::TestkitError>,
) -> Result<(), crate::TestkitError> {
    SLOT.get_or_try_init(|| async {
        let mut conn = connect().await?;
        let (max, reserved): (i64, i64) = sqlx::query_as(
            "SELECT current_setting('max_connections')::bigint, \
                    current_setting('superuser_reserved_connections')::bigint \
                    + current_setting('reserved_connections')::bigint",
        )
        .fetch_one(&mut conn)
        .await?;
        take(&mut conn, SLOT_CLASS, slots(max, reserved)).await?;
        Ok::<_, crate::TestkitError>(Mutex::new(conn))
    })
    .await
    .map(|_| ())
}

/// Takes one of the `slots` slots of `class` on `conn`, waiting while none
/// is free.
///
/// The search starts at a slot the backend's process id picks, so the
/// processes that look at once do not all try the same one.
async fn take(conn: &mut PgConnection, class: i32, slots: i32) -> Result<(), sqlx::Error> {
    loop {
        let taken: Option<i32> = sqlx::query_scalar(
            "SELECT slot FROM generate_series(0, $2 - 1) AS probe, \
                    LATERAL (SELECT (probe + pg_backend_pid()) % $2 AS slot) AS at \
             WHERE pg_try_advisory_lock($1, slot) LIMIT 1",
        )
        .bind(class)
        .bind(slots)
        .fetch_optional(&mut *conn)
        .await?;
        if taken.is_some() {
            return Ok(());
        }
        tokio::time::sleep(RETRY).await;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sqlx::{Connection as _, PgConnection};

    use super::{slots, take};

    /// A class no admission uses, so the test's slots are its own.
    const TEST_CLASS: i32 = 0x0EB2_5108;

    #[test]
    fn the_slots_follow_the_servers_connections() {
        assert_eq!(slots(400, 3), 22, "the testkit container");
        assert_eq!(slots(200, 3), 10);
        assert_eq!(slots(100, 3), 5, "a default server, as CI runs");
        assert_eq!(slots(10, 3), 1, "a server too small still admits one");
    }

    /// A test beyond the slots waits, and goes on when a slot is given back
    /// by the close of the connection that held it.
    #[tokio::test]
    async fn a_test_beyond_the_slots_waits_for_one_to_be_given_back() {
        let db = crate::db().await.expect("testkit database");
        let pool = db.pool();
        let connect = || async {
            PgConnection::connect(db.url())
                .await
                .expect("a connection to the clone")
        };
        let (mut first, mut second, mut third) =
            (connect().await, connect().await, connect().await);
        take(&mut first, TEST_CLASS, 2)
            .await
            .expect("the first slot");
        take(&mut second, TEST_CLASS, 2)
            .await
            .expect("the second slot");
        let waiting = tokio::spawn(async move {
            take(&mut third, TEST_CLASS, 2)
                .await
                .expect("a slot, once one is free");
            third
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            !waiting.is_finished(),
            "no slot is free, so the third waits"
        );
        drop(first);
        let third = tokio::time::timeout(Duration::from_secs(30), waiting)
            .await
            .expect("the third is admitted once the first connection closed")
            .expect("the waiting task ends");
        let held: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND classid = $1::oid",
        )
        .bind(i64::from(TEST_CLASS))
        .fetch_one(&pool)
        .await
        .expect("the held slots are counted");
        assert_eq!(held, 2, "both slots are held again");
        drop((second, third));
    }
}
