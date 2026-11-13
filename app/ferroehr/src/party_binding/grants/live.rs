// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`LiveGrants`]: this replica's copy of the grant rows and the caller
//! facts, from which every request resolves its caller's capabilities and
//! facts with no read of its own.
//!
//! The copy is read again when a notification on [`CHANNEL`] marks it stale,
//! when the listener is down, and after a request whose own commit changed a
//! grant or a fact, so the next request on every replica decides under the
//! change. A read is one indexed read of the two revisions; the grant rows
//! and every fact when the grant revision moved, and otherwise, when the
//! facts revision moved, one indexed read of the facts of the parties
//! stamped since ([`caller::store::FACTS`]). Which grants hold is judged at each request on its own
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

use crate::party_binding::grants::admission::Admission;
use crate::party_binding::grants::agreement::{Accessor, Agreement};
use crate::party_binding::grants::caller::store::{Changed, FactRow};
use crate::party_binding::grants::caller::{self, CallerFacts, CareTeam, FactKind, FactValue};
use crate::party_binding::grants::capabilities::{
    Actor, Capabilities, Client, ClientState, Credentials, Grant, Provider,
};
use crate::party_binding::grants::operation::{GrantCeiling, Operation};
use crate::party_binding::grants::store::{self, CHANNEL, CapabilityRow, Loaded};
use crate::party_binding::grants::trust::{AgreementPosture, FederationTrust};
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

/// The caller facts as this replica holds them.
#[derive(Debug, Default)]
struct HeldFacts {
    /// The facts revision read; `None` before the first read.
    revision: Option<i64>,
    /// The facts about each party.
    by_party: BTreeMap<Uuid, Arc<[FactRow]>>,
}

impl HeldFacts {
    /// These facts with `changed` applied: each party read holds the facts
    /// read, and a party read with none holds none.
    fn with(&self, changed: Changed) -> Self {
        let mut by_party = self.by_party.clone();
        for (party, facts) in changed.parties {
            if facts.is_empty() {
                by_party.remove(&party);
            } else {
                by_party.insert(party, facts.into());
            }
        }
        Self {
            revision: Some(changed.revision),
            by_party,
        }
    }
}

/// This replica's copy of the grant rows and the caller facts, and the
/// ceiling it honours the grants under.
pub struct LiveGrants {
    pool: PgPool,
    ceiling: RwLock<Arc<GrantCeiling>>,
    federation: RwLock<Arc<FederationTrust>>,
    held: RwLock<Arc<Held>>,
    facts: RwLock<Arc<HeldFacts>>,
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
            federation: RwLock::new(Arc::default()),
            held: RwLock::new(Arc::default()),
            facts: RwLock::new(Arc::default()),
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

    /// Judge the federation agreements against `trust`: the `federation`
    /// issuers and how long this node keeps its access records.
    pub fn set_federation(&self, trust: FederationTrust) {
        *self
            .federation
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(trust);
    }

    /// What the federation agreements are judged against.
    #[must_use]
    pub fn federation(&self) -> Arc<FederationTrust> {
        self.federation
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

    /// Read the copy again in a detached task whenever a notification marks
    /// it stale, and at least every `every` while the listener is down, so a
    /// commit of a request that resolves no caller, or of no request at all,
    /// decides under the grants in force (whether it writes its outbox row)
    /// without a read on its own path.
    pub fn spawn_refresher(self: &Arc<Self>, every: std::time::Duration) {
        tokio::spawn(refresher(Arc::clone(self), every));
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

    fn held_facts(&self) -> Arc<HeldFacts> {
        self.facts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The facts revision this replica holds; `None` before its first read.
    #[must_use]
    pub fn facts_revision(&self) -> Option<i64> {
        self.held_facts().revision
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

    /// Read the revisions; every grant row and fact when the grant revision
    /// moved past the one held, else the facts of the parties stamped since
    /// the facts revision held when that moved.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the party store does not answer; the copy
    /// held stays, and the next request reads again.
    pub async fn refresh(&self) -> Result<(), GrantsUnavailable> {
        let read = async {
            let mut conn = self.pool.acquire().await?;
            let (revision, facts_revision) = store::revisions(&mut conn).await?;
            if Some(revision) != self.revision() {
                let _installing = self.installing.lock().await;
                let loaded = store::load(&mut conn).await?;
                self.install(loaded);
            } else if Some(facts_revision) != self.facts_revision() {
                let _installing = self.installing.lock().await;
                let since = self.facts_revision();
                let changed = read_facts(&mut conn, since).await?;
                let held = self.held_facts();
                if held.revision.is_none_or(|held| held < changed.revision) {
                    *self.facts.write().unwrap_or_else(PoisonError::into_inner) =
                        Arc::new(held.with(changed));
                }
            }
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
        *self.facts.write().unwrap_or_else(PoisonError::into_inner) =
            Arc::new(HeldFacts::default().with(loaded.facts));
        *self.held.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(Held {
            revision: Some(loaded.revision),
            clients,
            capabilities,
        });
    }

    /// The capabilities of `actor` at `now`, from the copy held: the
    /// operations the `ROLE`s it performs grant that hold on the date of
    /// `now` in UTC and at `now`, of an operation the ceiling admits, and its
    /// caller facts in force then ([`Self::facts_of`]).
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
        Capabilities::new(client, grants).with_facts(self.facts_of(performer, now))
    }

    /// The caller facts of the party `party` at `now`, from the copy held:
    /// the clearances and purposes of the capabilities of the `ROLE`s it
    /// performs that hold on the date of `now` in UTC and at `now`, and the
    /// care teams it is a member of on that date.
    #[must_use]
    pub fn facts_of(&self, party: Uuid, now: jiff::Timestamp) -> CallerFacts {
        let held = self.held_facts();
        let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
        let (mut clearances, mut purposes, mut care_teams) = (Vec::new(), Vec::new(), Vec::new());
        for row in held
            .by_party
            .get(&party)
            .into_iter()
            .flat_map(|rows| rows.iter())
        {
            if !row.term.holds(today, now) {
                continue;
            }
            match (row.kind, &row.value) {
                (FactKind::Clearance, FactValue::Code(code)) => clearances.push(code.clone()),
                (FactKind::Purpose, FactValue::Code(code)) => purposes.push(code.clone()),
                (FactKind::MemberOf, FactValue::Team(team)) => {
                    care_teams.extend(row.term.dates.map(|dates| CareTeam::new(*team, dates)));
                }
                _ => {}
            }
        }
        CallerFacts::new(clearances, purposes, care_teams)
    }

    /// The care teams and units the subject `subject` is under the care of,
    /// from the copy held: each `under_care_of` relationship of its live
    /// `PERSON` head with the dates it holds, whether or not it holds today,
    /// so the evaluator judges a discharge and the grace after it.
    #[must_use]
    pub fn under_care_of(&self, subject: Uuid) -> Vec<CareTeam> {
        let held = self.held_facts();
        let mut out: Vec<CareTeam> = held
            .by_party
            .get(&subject)
            .into_iter()
            .flat_map(|rows| rows.iter())
            .filter(|row| row.kind == FactKind::UnderCareOf)
            .filter(|row| matches!(row.term.lifecycle_state.as_str(), "532" | "553"))
            .filter_map(|row| match (row.value.clone(), row.term.dates) {
                (FactValue::Team(team), Some(dates)) => Some(CareTeam::new(team, dates)),
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Every federation agreement in force at `now`, once per client
    /// identifier of its partner, from the copy held: what the boot line and
    /// `/management/info` list.
    #[must_use]
    pub fn agreements(&self, now: jiff::Timestamp) -> Vec<AgreementPosture> {
        let held = self.held();
        let ceiling = self.ceiling();
        let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
        let mut out = Vec::new();
        for ((issuer, client_id), client) in &held.clients {
            if !client.state.grants() {
                continue;
            }
            for row in held.capabilities.get(&client.party).into_iter().flatten() {
                let Some(accessor) = in_force(row, &ceiling, today, now).and_then(Accessor::of)
                else {
                    continue;
                };
                let grant = Grant::new(
                    accessor.operation(),
                    row.role_id,
                    row.credentials.clone().map(Arc::new),
                );
                // NOTE: an agreement that does not read is listed without its terms,
                // and the admission refuses every caller under it.
                let agreement = Agreement::read(accessor, grant.credentials()).ok();
                out.push(AgreementPosture {
                    issuer: ferrohealth_secret::secret_url::SecretUrl::new(issuer.as_str())
                        .redacted(),
                    client_id: client_id.clone(),
                    accessor,
                    name: agreement
                        .as_ref()
                        .map(|agreement| agreement.name.clone())
                        .unwrap_or_default(),
                    legal_ground: agreement
                        .and_then(|agreement| agreement.legal_ground)
                        .map(|ground| ground.to_string()),
                });
            }
        }
        out
    }

    /// Logs the federation agreements in force: one line each, a `warn` for
    /// one that runs without a named person.
    pub fn log_agreements(&self) {
        for agreement in self.agreements(jiff::Timestamp::now()) {
            let ground = agreement.legal_ground.as_deref().unwrap_or("-");
            if agreement.accessor == Accessor::Organisation {
                tracing::warn!(
                    issuer = %agreement.issuer,
                    client_id = %agreement.client_id,
                    agreement = %agreement.name,
                    accessor = %agreement.accessor,
                    legal_ground = %ground,
                    "a federation agreement admits an organisation without a named person"
                );
            } else {
                tracing::info!(
                    issuer = %agreement.issuer,
                    client_id = %agreement.client_id,
                    agreement = %agreement.name,
                    accessor = %agreement.accessor,
                    legal_ground = %ground,
                    "federation agreement in force"
                );
            }
        }
    }

    /// The admission rules in force at `now` for the first logins of the
    /// token issuer `issuer`, from the copy held: each
    /// [`Operation::CreateAtLogin`] grant naming it that holds on the date of
    /// `now` in UTC and at `now`, under the ceiling, whoever performs its
    /// `ROLE`.
    #[must_use]
    pub fn admissions(&self, issuer: &str, now: jiff::Timestamp) -> Vec<Admission> {
        let held = self.held();
        let ceiling = self.ceiling();
        let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
        held.capabilities
            .values()
            .flatten()
            .filter(|row| in_force(row, &ceiling, today, now) == Some(Operation::CreateAtLogin))
            // NOTE: admission::check refuses the commit of credentials that do not
            // read, so a stored one that does not is never honoured; our own design.
            .filter_map(|row| Admission::read(Credentials::of(row.credentials.as_ref())).ok())
            .filter(|admission| admission.names(issuer))
            .collect()
    }
}

impl LiveGrants {
    /// Every grant of `operation` in force at `now`, of any performer, with
    /// its performer, from the copy held: what a store-wide reading such as
    /// the registered origins or the change-feed readers ranges over. A
    /// performer that is a client `ACTOR` in a state that holds no grant
    /// holds none here either.
    #[must_use]
    pub fn all_in_force(&self, operation: Operation, now: jiff::Timestamp) -> Vec<(Uuid, Grant)> {
        let held = self.held();
        let ceiling = self.ceiling();
        let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
        let silenced: std::collections::BTreeSet<Uuid> = held
            .clients
            .values()
            .filter(|client| !client.state.grants())
            .map(|client| client.party)
            .collect();
        held.capabilities
            .iter()
            .filter(|(performer, _)| !silenced.contains(performer))
            .flat_map(|(performer, rows)| rows.iter().map(move |row| (*performer, row)))
            .filter(|(_, row)| in_force(row, &ceiling, today, now) == Some(operation))
            .map(|(performer, row)| {
                (
                    performer,
                    Grant::new(
                        operation,
                        row.role_id,
                        row.credentials.clone().map(Arc::new),
                    ),
                )
            })
            .collect()
    }

    /// Every stored grant of `operation`, of any performer, whether or not it
    /// is in force or above the ceiling: what must still be honoured once a
    /// grant has ended, such as the erasure of the contacts of the Users a
    /// former SCIM client deleted.
    #[must_use]
    pub fn all_stored(&self, operation: Operation) -> Vec<Grant> {
        self.held()
            .capabilities
            .values()
            .flatten()
            .filter(|row| Operation::of(&row.operation) == Some(operation))
            .map(|row| {
                Grant::new(
                    operation,
                    row.role_id,
                    row.credentials.clone().map(Arc::new),
                )
            })
            .collect()
    }
}

/// The refresher loop of [`LiveGrants::spawn_refresher`].
#[expect(
    clippy::infinite_loop,
    reason = "a detached task that lives as long as the runtime"
)]
async fn refresher(grants: Arc<LiveGrants>, every: std::time::Duration) {
    loop {
        let seen = grants.listened.notifications();
        let _notified = grants.listened.notified_after(seen, every).await;
        if let Err(error) = grants.refresh_if_stale().await {
            tracing::warn!(
                error = %crate::error::ErrorChain::new(&error),
                "the policy grants could not be read; the next notification reads them again"
            );
        }
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
    row.term.holds(today, now).then_some(operation)
}

/// The facts of the parties stamped after `since` (every party for `None`),
/// read in one repeatable-read snapshot on `conn`.
async fn read_facts(
    conn: &mut sqlx::PgConnection,
    since: Option<i64>,
) -> Result<Changed, StorageError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let changed = caller::store::load(&mut tx, since).await?;
    tx.commit().await?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party_binding::grants::store::{Dates, Term};

    fn date(text: &str) -> jiff::civil::Date {
        text.parse().expect("a date")
    }

    fn row(code: &str) -> CapabilityRow {
        CapabilityRow {
            role_id: Uuid::nil(),
            performer_id: Uuid::nil(),
            operation: code.to_owned(),
            term: Term {
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

    #[tokio::test]
    async fn a_change_is_tracked_inside_the_request_only() {
        let ((), changed) = tracking(async { note_change() }).await;
        assert!(changed);
        let ((), changed) = tracking(async {}).await;
        assert!(!changed);
        note_change();
    }
}
