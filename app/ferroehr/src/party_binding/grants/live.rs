// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`LiveGrants`]: this replica's copy of the grant rows, from which every
//! request resolves its caller's capabilities with no read of its own.
//!
//! The copy is read again when a notification on [`CHANNEL`] marks it stale,
//! when the listener is down, and after a request whose own commit changed a
//! grant, so the next request on every replica decides under the change. A
//! read is one indexed read of the revision, and the rows only when the
//! revision moved. Which grants hold is judged at each request on its own
//! date and instant, and a grant of an operation `[authz.grants] allowed` does not
//! admit is never honoured. No openEHR spec governs operational policy: our
//! own design.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use sqlx::PgPool;
use uuid::Uuid;

use crate::party_binding::grants::capabilities::{
    Actor, Capabilities, Client, ClientState, Grant, Provider,
};
use crate::party_binding::grants::operation::{GrantCeiling, Operation};
use crate::party_binding::grants::store::{self, CHANNEL, CapabilityRow, Loaded};
use crate::policy_register::listen::Listened;
use crate::storage::error::StorageError;

tokio::task_local! {
    /// Set when a commit of the running request changed a grant row.
    static CHANGED: AtomicBool;
}

/// Records that the running request's commit changed a grant row, so the
/// request reads the grants again before it answers.
pub(crate) fn note_change() {
    // NOTE: tokio LocalKey::try_with, outside a tracked request (a refill, a
    // test) there is no request to answer, and the notification covers it.
    match CHANGED.try_with(|changed| changed.store(true, Ordering::Release)) {
        Ok(()) | Err(_) => {}
    }
}

/// Runs `fut`, returning its output and whether a commit inside it changed a
/// grant row.
pub async fn tracking<F: Future>(fut: F) -> (F::Output, bool) {
    CHANGED
        .scope(AtomicBool::new(false), async {
            let output = fut.await;
            let changed = CHANGED.with(|changed| changed.load(Ordering::Acquire));
            (output, changed)
        })
        .await
}

/// Why the grants could not be read.
#[derive(Debug, thiserror::Error)]
#[error("the policy grants could not be read")]
pub struct GrantsUnavailable(#[from] StorageError);

impl GrantsUnavailable {
    /// The store failure.
    #[must_use]
    pub fn into_inner(self) -> StorageError {
        self.0
    }
}

/// The grant rows as this replica holds them.
#[derive(Debug, Default)]
struct Held {
    /// The revision read; `None` before the first read.
    revision: Option<i64>,
    /// Each client `ACTOR` by `(issuer, client_id)`.
    clients: BTreeMap<(String, String), Client>,
    /// The capabilities of each performer.
    capabilities: BTreeMap<Uuid, Vec<CapabilityRow>>,
}

/// This replica's copy of the grant rows and the ceiling it honours them
/// under.
pub struct LiveGrants {
    pool: PgPool,
    ceiling: RwLock<Arc<GrantCeiling>>,
    held: RwLock<Arc<Held>>,
    /// Serializes the reads that install a new revision.
    installing: tokio::sync::Mutex<()>,
    /// The listener on [`CHANNEL`].
    listened: Arc<Listened>,
}

impl fmt::Debug for LiveGrants {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveGrants")
            .field("revision", &self.held().revision)
            .finish_non_exhaustive()
    }
}

impl LiveGrants {
    /// The grants over the party pool `pool`, under the default ceiling, read
    /// on the first request.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            listened: Arc::new(Listened::new(pool.clone(), CHANNEL)),
            pool,
            ceiling: RwLock::new(Arc::new(GrantCeiling::default())),
            held: RwLock::new(Arc::default()),
            installing: tokio::sync::Mutex::new(()),
        }
    }

    /// Honour the grants under `ceiling`: `[authz.grants] allowed`.
    pub fn set_ceiling(&self, ceiling: GrantCeiling) {
        *self.ceiling.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(ceiling);
    }

    /// The ceiling in force.
    #[must_use]
    pub fn ceiling(&self) -> Arc<GrantCeiling> {
        self.ceiling
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The listener that says when the copy must be read again;
    /// [`Listened::spawn_listener`] starts it.
    #[must_use]
    pub fn listened(&self) -> &Arc<Listened> {
        &self.listened
    }

    /// The revision this replica holds; `None` before its first read.
    #[must_use]
    pub fn revision(&self) -> Option<i64> {
        self.held().revision
    }

    fn held(&self) -> Arc<Held> {
        self.held
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Read the grant rows when a notification marked them stale or the
    /// listener is down; otherwise the copy stands, and the request touches
    /// no database.
    ///
    /// # Errors
    /// As [`Self::refresh`].
    pub async fn refresh_if_stale(&self) -> Result<(), GrantsUnavailable> {
        if self.listened.must_read() {
            self.refresh().await?;
        }
        Ok(())
    }

    /// Read the revision, and the rows when it moved past the one held.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the party store does not answer; the copy
    /// held stays, and the next request reads again.
    pub async fn refresh(&self) -> Result<(), GrantsUnavailable> {
        let read = async {
            let mut conn = self.pool.acquire().await?;
            if Some(store::revision(&mut conn).await?) == self.revision() {
                return Ok(());
            }
            let _installing = self.installing.lock().await;
            let loaded = store::load(&mut conn).await?;
            self.install(loaded);
            Ok::<(), StorageError>(())
        };
        match read.await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.listened.mark_stale();
                Err(GrantsUnavailable(error))
            }
        }
    }

    /// Install `loaded` as the copy held, logging the stored grants the
    /// ceiling does not admit.
    fn install(&self, loaded: Loaded) {
        let ceiling = self.ceiling();
        let above: Vec<&str> = loaded
            .capabilities
            .iter()
            .map(|row| row.operation.as_str())
            .filter(|code| !ceiling.allows_code(code))
            .collect();
        if !above.is_empty() {
            tracing::warn!(
                count = above.len(),
                operations = %above.join(", "),
                "stored grants are above [authz.grants] allowed and are not honoured"
            );
        }
        let clients = loaded
            .clients
            .into_iter()
            .map(|row| {
                let client = Client {
                    party: row.party_id,
                    state: ClientState::of(&row.lifecycle_state),
                    provider: row.provider_id.map(|party| Provider {
                        party,
                        identifier: row.provider,
                    }),
                };
                ((row.issuer, row.client_id), client)
            })
            .collect();
        let mut capabilities: BTreeMap<Uuid, Vec<CapabilityRow>> = BTreeMap::new();
        for row in loaded.capabilities {
            capabilities.entry(row.performer_id).or_default().push(row);
        }
        *self.held.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(Held {
            revision: Some(loaded.revision),
            clients,
            capabilities,
        });
    }

    /// The capabilities of `actor` at `now`, from the copy held: the
    /// operations the `ROLE`s it performs grant that hold on the date of
    /// `now` in UTC and at `now`, of an operation the ceiling admits.
    #[must_use]
    pub fn capabilities(&self, actor: Actor<'_>, now: jiff::Timestamp) -> Capabilities {
        let held = self.held();
        let (client, performer) = match actor {
            Actor::Client { issuer, client_id } => {
                let Some(client) = held
                    .clients
                    .get(&(issuer.to_owned(), client_id.to_owned()))
                    .cloned()
                else {
                    return Capabilities::default();
                };
                if !client.state.grants() {
                    return Capabilities::new(Some(client), Vec::new());
                }
                let party = client.party;
                (Some(client), party)
            }
            Actor::Party(party) => (None, party),
        };
        let ceiling = self.ceiling();
        let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
        let grants = held
            .capabilities
            .get(&performer)
            .into_iter()
            .flatten()
            .filter_map(|row| {
                let operation = in_force(row, &ceiling, today, now)?;
                Some(Grant::new(
                    operation,
                    row.role_id,
                    row.credentials.clone().map(Arc::new),
                ))
            })
            .collect();
        Capabilities::new(client, grants)
    }
}

/// The operation of the stored capability `row` when it is in force on `today` and
/// at `now` under `ceiling`.
fn in_force(
    row: &CapabilityRow,
    ceiling: &GrantCeiling,
    today: jiff::civil::Date,
    now: jiff::Timestamp,
) -> Option<Operation> {
    let operation = Operation::of(&row.operation).filter(|operation| ceiling.allows(*operation))?;
    let live = matches!(row.lifecycle_state.as_str(), "532" | "553");
    let dated = row.role_dates.is_some_and(|dates| dates.holds(today))
        && row.dates.is_some_and(|dates| dates.holds(today));
    let timed = row.not_before.is_none_or(|before| before <= now)
        && row.not_after.is_none_or(|after| now <= after);
    (live && dated && timed).then_some(operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party_binding::grants::store::Dates;

    fn date(text: &str) -> jiff::civil::Date {
        text.parse().expect("a date")
    }

    fn row(code: &str) -> CapabilityRow {
        CapabilityRow {
            role_id: Uuid::nil(),
            performer_id: Uuid::nil(),
            operation: code.to_owned(),
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
            credentials: None,
        }
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
        timed.not_before = Some("2026-12-02T13:00:00Z".parse().expect("an instant"));
        assert_eq!(in_force(&timed, &ceiling, today, now), None);
        timed.not_before = None;
        timed.not_after = Some("2026-12-02T11:00:00Z".parse().expect("an instant"));
        assert_eq!(in_force(&timed, &ceiling, today, now), None);
        let mut empty = relay.clone();
        empty.role_dates = None;
        assert_eq!(in_force(&empty, &ceiling, today, now), None);
        let mut inactive = relay;
        inactive.lifecycle_state = "800".to_owned();
        assert_eq!(in_force(&inactive, &ceiling, today, now), None);
        assert_eq!(
            in_force(&row("import_everything"), &ceiling, today, now),
            None
        );
    }

    #[tokio::test]
    async fn a_change_is_tracked_inside_the_request_only() {
        let ((), changed) = tracking(async { note_change() }).await;
        assert!(changed);
        let ((), changed) = tracking(async {}).await;
        assert!(!changed);
        note_change();
    }
}
