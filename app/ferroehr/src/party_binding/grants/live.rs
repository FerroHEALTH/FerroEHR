// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`LiveGrants`]: this replica's copy of the grant rows and the caller
//! facts, from which every request resolves its caller's capabilities and
//! facts with no read of its own.
//!
//! The care relationships of subjects are no caller facts: the care copy
//! ([`crate::party_binding::grants::care`]) holds them in caches of a fixed
//! bound, read on a miss (#5615).
//!
//! The copy is read again when a notification on [`CHANNEL`] marks the
//! grants stale, when the listener is down, after a request whose own commit
//! changed a grant or a fact, and when a request's read of the store's
//! revisions finds a caller facts stamp the copy's snapshot did not see
//! (#6074); a facts change sends no notification. A read is one indexed read
//! of the revisions; the grant rows and every fact when the grant revision
//! moved, and otherwise, when a stamp is unseen, one indexed read of the facts
//! of the parties stamped by the transactions the copy's snapshot did not see
//! ([`caller::store::FACTS`]), after which the copy holds the snapshot of that
//! read. Which grants hold is judged at each request on its own
//! date and instant, and a grant of an operation `[authz.grants] allowed` does not
//! admit is never honoured. No openEHR spec governs operational policy: our
//! own design.

use crate::db::domain::Domain;
use futures::FutureExt as _;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use sqlx::PgPool;
use uuid::Uuid;

use self::revisions::Revisions;
use crate::db::acquire::acquire;
use crate::db::snapshot::Snapshot;
use crate::party_binding::grants::admission::Admission;
use crate::party_binding::grants::agreement::{Accessor, Agreement};
use crate::party_binding::grants::caller::care_of::ProviderKey;
use crate::party_binding::grants::caller::facts::{CallerFacts, Kin};
use crate::party_binding::grants::caller::store::{Changed, FactRow};
use crate::party_binding::grants::caller::{self, CareTeam, FactKind, FactValue};
use crate::party_binding::grants::capabilities::{
    Actor, Capabilities, Client, ClientState, Credentials, Grant, Provider,
};
use crate::party_binding::grants::care::CareCopy;
use crate::party_binding::grants::operation::{GrantCeiling, Operation};
use crate::party_binding::grants::store::{self, CHANNEL, CapabilityRow, Loaded};
use crate::party_binding::grants::trust::{AgreementPosture, FederationTrust};
use crate::policy_register::listen::{Listened, Subscriber};
use crate::storage::error::StorageError;
use openehr_rm::draft::access_control::terminology::ForeignCode;

mod held;
pub mod revisions;
#[cfg(test)]
mod sharing;
mod subjects;

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
pub fn tracking<F: Future>(fut: F) -> impl Future<Output = (F::Output, bool)> {
    CHANGED.scope(
        AtomicBool::new(false),
        fut.map(|output| {
            let changed = CHANGED.with(|changed| changed.load(Ordering::Acquire));
            (output, changed)
        }),
    )
}

/// Why the grants could not be read.
#[derive(Debug, thiserror::Error)]
#[error("the policy grants could not be read")]
pub struct GrantsUnavailable(#[from] StorageError);

impl From<GrantsUnavailable> for crate::error::ServiceError {
    /// A copy that cannot be read answers `503`: the request is refused, never
    /// decided without the facts it needs.
    fn from(error: GrantsUnavailable) -> Self {
        Self::overloaded("the access facts could not be read", error)
    }
}

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
    /// Each client `ACTOR` by issuer, then `client_id`, so a request looks one
    /// up by its borrowed identifiers.
    clients: BTreeMap<String, BTreeMap<String, Arc<Client>>>,
    /// The capabilities of each performer.
    capabilities: BTreeMap<Uuid, Vec<CapabilityRow>>,
}

/// The caller facts as this replica holds them.
#[derive(Debug, Default)]
struct HeldFacts {
    /// The snapshot of the read they were brought to; `None` before the first.
    snapshot: Option<Arc<Snapshot>>,
    /// How many reads were installed, so a read of the store's revisions
    /// judged against one install holds for every later one.
    generation: u64,
    /// The number of the facts read installed; `0` before the first.
    read: u64,
    /// The facts about each party.
    by_party: BTreeMap<Uuid, Arc<[FactRow]>>,
}

impl HeldFacts {
    /// These facts with `changed` applied: each party read holds the facts
    /// read, and a party read with none holds none.
    fn with(&self, changed: Changed, read: u64) -> Self {
        Self::from_parts(self.by_party.clone(), self.generation, changed, read)
    }

    /// The facts `changed` reads whole, installed after these.
    fn whole(&self, changed: Changed, read: u64) -> Self {
        Self::from_parts(BTreeMap::new(), self.generation, changed, read)
    }

    fn from_parts(
        mut by_party: BTreeMap<Uuid, Arc<[FactRow]>>,
        generation: u64,
        changed: Changed,
        read: u64,
    ) -> Self {
        for (party, facts) in changed.parties {
            if facts.is_empty() {
                by_party.remove(&party);
            } else {
                by_party.insert(party, facts.into());
            }
        }
        Self {
            snapshot: Some(changed.snapshot),
            generation: generation.saturating_add(1),
            read,
            by_party,
        }
    }
}

/// This replica's copy of the grant rows and the caller facts, and the
/// ceiling it honours the grants under.
pub struct LiveGrants {
    pool: PgPool,
    /// The `PARTY_REF.namespace` this CDR names its parties under
    /// (`[authz.party_binding] namespace`).
    namespace: RwLock<Arc<str>>,
    ceiling: RwLock<Arc<GrantCeiling>>,
    federation: RwLock<Arc<FederationTrust>>,
    held: RwLock<Arc<Held>>,
    facts: RwLock<Arc<HeldFacts>>,
    /// Serializes the reads that install a new revision.
    installing: tokio::sync::Mutex<()>,
    /// The facts reads this copy started, each numbered by it as it starts,
    /// so a read of the store's revisions knows which reads began after it
    /// returned.
    facts_started: Arc<AtomicU64>,
    /// Holds every read of the copy back, so a test meets a copy that never
    /// catches up with the store (#5806).
    #[cfg(test)]
    held_back: AtomicBool,
    /// The full loads this copy ran, so a test counts them (#6343).
    #[cfg(test)]
    loads: std::sync::atomic::AtomicUsize,
    /// The listener on [`CHANNEL`].
    listened: Arc<Listened>,
    /// The subjects' care relationships and the teams' patients held.
    care: Arc<CareCopy>,
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
        let care = Arc::new(CareCopy::new());
        let dropping: Arc<dyn Subscriber> = Arc::<CareCopy>::clone(&care);
        Self {
            listened: Arc::new(Listened::with_subscriber(CHANNEL, Domain::Party, dropping)),
            care,
            pool,
            namespace: RwLock::new(Arc::from(
                crate::config::binding::PartyBindingConfig::default()
                    .namespace
                    .as_str(),
            )),
            ceiling: RwLock::new(Arc::new(GrantCeiling::default())),
            federation: RwLock::new(Arc::default()),
            held: RwLock::new(Arc::default()),
            facts: RwLock::new(Arc::default()),
            installing: tokio::sync::Mutex::new(()),
            facts_started: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            held_back: AtomicBool::new(false),
            #[cfg(test)]
            loads: std::sync::atomic::AtomicUsize::new(0),
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

    /// Name this CDR's parties under `namespace`, `[authz.party_binding]
    /// namespace`: a subject reference under it names a `PERSON` by its
    /// container ([`Self::person_of`]).
    pub fn set_namespace(&self, namespace: &str) {
        *self
            .namespace
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Arc::from(namespace);
    }

    /// The namespace this CDR names its parties under.
    #[must_use]
    pub fn namespace(&self) -> Arc<str> {
        self.namespace
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
    /// [`crate::policy_register::listen::Listeners::spawn`] starts it.
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

    /// Read the grant rows when a notification marked them stale or the
    /// listener is down; otherwise the copy stands, and the caller touches
    /// no database. A request decides on [`Self::confirm`].
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
    /// moved past the one held, else the facts of the parties stamped by
    /// transactions the snapshot held did not see, when the store holds any.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the party store does not answer; the copy
    /// held stays, and the next request reads again.
    pub async fn refresh(&self) -> Result<(), GrantsUnavailable> {
        #[cfg(test)]
        if self.held_back.load(Ordering::Acquire) {
            return Ok(());
        }
        let started = std::time::Instant::now();
        let mut kind = "unchanged";
        let read = async {
            let mut conn = acquire(&self.pool).await?;
            let Revisions {
                grants: revision,
                facts: facts_read,
                ..
            } = store::revisions(&mut conn, Some(&self.held_snapshot())).await?;
            // NOTE: our own design (#6343), a refresh queued behind one that installed the
            // revision it read reads nothing more.
            if Some(revision) != self.revision() {
                let _installing = crate::db::acquire::awaiting(self.installing.lock()).await;
                if self.revision().is_some_and(|held| held >= revision) {
                    return Ok(());
                }
                kind = "grants";
                #[cfg(test)]
                self.loads.fetch_add(1, Ordering::AcqRel);
                let read = self.facts_read_starts();
                let loaded = store::load(&mut conn).await?;
                self.install(loaded, read);
            } else if !self.facts_hold(&facts_read) {
                let _installing = crate::db::acquire::awaiting(self.installing.lock()).await;
                if self.facts_hold(&facts_read) {
                    return Ok(());
                }
                kind = "facts";
                let since = self.facts_snapshot();
                let read = self.facts_read_starts();
                let changed = read_facts(&mut conn, since.as_deref()).await?;
                // NOTE: our own design (#5806, #6074), the care copy applies what changed before the
                // snapshot moves, so a request that finds the copy current reads no older care.
                self.care.apply(&changed).await;
                let held = self.held_facts();
                if held
                    .snapshot
                    .as_ref()
                    .is_none_or(|held| changed.snapshot.covers(held))
                {
                    *self.facts.write().unwrap_or_else(PoisonError::into_inner) =
                        Arc::new(held.with(changed, read));
                }
            }
            Ok::<(), StorageError>(())
        };
        let read = read.await;
        crate::telemetry::access_path::metrics()
            .grants_refresh_duration
            .record(
                crate::telemetry::access_path::seconds_since(started),
                &[
                    opentelemetry::KeyValue::new("kind", kind),
                    crate::telemetry::access_path::outcome(&read),
                ],
            );
        match read {
            Ok(()) => Ok(()),
            Err(error) => {
                self.listened.mark_stale();
                Err(GrantsUnavailable(error))
            }
        }
    }

    /// The number of a facts read that starts now.
    fn facts_read_starts(&self) -> u64 {
        self.facts_started
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1)
    }

    /// Install `loaded`, read by the facts read numbered `read`, as the copy
    /// held, logging the stored grants the ceiling does not admit.
    fn install(&self, loaded: Loaded, read: u64) {
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
        let mut clients: BTreeMap<String, BTreeMap<String, Arc<Client>>> = BTreeMap::new();
        for row in loaded.clients {
            let client = Client {
                party: row.party_id,
                state: ClientState::of(&row.lifecycle_state),
                provider: row.provider_id.map(|party| Provider {
                    party,
                    identifier: row.provider,
                }),
            };
            clients
                .entry(row.issuer)
                .or_default()
                .insert(row.client_id, Arc::new(client));
        }
        let mut capabilities: BTreeMap<Uuid, Vec<CapabilityRow>> = BTreeMap::new();
        for row in loaded.capabilities {
            capabilities.entry(row.performer_id).or_default().push(row);
        }
        self.care.drop_all();
        let facts = self.held_facts().whole(loaded.facts, read);
        *self.facts.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(facts);
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
                    .get(issuer)
                    .and_then(|clients| clients.get(client_id))
                    .map(Arc::clone)
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
                Some(Grant::of_row(operation, row))
            })
            .collect();
        let provider = client.as_ref().and_then(|client| client.provider.as_ref());
        let party = provider.map(|provider| provider.party);
        let groups = party.map_or_else(Vec::new, |party| self.memberships(party, now));
        let types = party.map_or_else(Vec::new, |party| self.organisation_types(party));
        let (namespace, identifier) = (self.namespace(), provider.and_then(Provider::key));
        let units = party.iter().chain(&groups);
        let providers: Vec<ProviderKey> = units
            .map(|unit| ProviderKey::new(&*namespace, unit.to_string()))
            .chain(identifier)
            .collect();
        let facts = self
            .facts_of(performer, now)
            .with_provider_groups(groups)
            .with_organisation_types(types)
            .with_providers(providers);
        Capabilities::new(client, grants).with_facts(facts)
    }

    /// The `GROUP`s the `ORGANISATION` `organisation` is a member of at
    /// `now`, from the copy held: each membership (`MBR`) relationship of its live
    /// head that holds on the date of `now` in UTC.
    #[must_use]
    pub fn memberships(&self, organisation: Uuid, now: jiff::Timestamp) -> Vec<Uuid> {
        let held = self.held_facts();
        let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
        held.by_party
            .get(&organisation)
            .into_iter()
            .flat_map(|rows| rows.iter())
            .filter(|row| row.kind == FactKind::MemberOf && row.term.holds(today, now))
            .filter_map(|row| match row.value {
                FactValue::Team(group) => Some(group),
                FactValue::Code(_) | FactValue::Care(_) | FactValue::Kin(_) => None,
            })
            .collect()
    }

    /// The types the coded name of the `ORGANISATION` `organisation` names, from the copy held.
    #[must_use]
    pub fn organisation_types(&self, organisation: Uuid) -> Vec<ForeignCode> {
        self.held_facts()
            .by_party
            .get(&organisation)
            .into_iter()
            .flat_map(|rows| rows.iter())
            .filter(|row| row.kind == FactKind::OrganisationType)
            .filter_map(|row| match &row.value {
                FactValue::Code(code) => Some(code.clone()),
                FactValue::Team(_) | FactValue::Care(_) | FactValue::Kin(_) => None,
            })
            .collect()
    }

    /// The caller facts of the party `party` at `now`, from the copy held:
    /// the `ROLE`s it performs that hold on the date of `now` in UTC with
    /// their roles, the care teams it is a member of on that date by a
    /// membership it did not write itself, and the persons it is kin to.
    #[must_use]
    pub fn facts_of(&self, party: Uuid, now: jiff::Timestamp) -> CallerFacts {
        let held = self.held_facts();
        let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
        let (mut roles, mut performed, mut care_teams, mut kin) = (vec![], vec![], vec![], vec![]);
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
                (FactKind::Role, FactValue::Code(code)) => {
                    roles.push(code.clone());
                    performed.push(row.holder);
                }
                // NOTE: draft decision procedure, Writing Relationships, a caller is never
                // admitted on a relationship it wrote naming itself, a membership included.
                (FactKind::MemberOf, FactValue::Team(team)) if row.writer != Some(party) => {
                    care_teams.extend(row.term.dates.map(|dates| CareTeam::new(*team, dates)));
                }
                (FactKind::RelatedTo, FactValue::Kin(fact)) => kin.push(Kin::from(fact)),
                _ => {}
            }
        }
        CallerFacts::new(roles, care_teams)
            .with_performed(performed)
            .with_kin(kin)
            .with_party(party)
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
        let clients = held.clients.iter().flat_map(|(issuer, clients)| {
            clients.iter().map(move |(id, client)| (issuer, id, client))
        });
        for (issuer, client_id, client) in clients {
            if !client.state.grants() {
                continue;
            }
            for row in held.capabilities.get(&client.party).into_iter().flatten() {
                let Some(accessor) = in_force(row, &ceiling, today, now).and_then(Accessor::of)
                else {
                    continue;
                };
                let grant = Grant::of_row(accessor.operation(), row);
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
            .filter_map(|row| Admission::read(Credentials::of(row.credentials.as_deref())).ok())
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
            .flat_map(BTreeMap::values)
            .filter(|client| !client.state.grants())
            .map(|client| client.party)
            .collect();
        held.capabilities
            .iter()
            .filter(|(performer, _)| !silenced.contains(performer))
            .flat_map(|(performer, rows)| rows.iter().map(move |row| (*performer, row)))
            .filter(|(_, row)| in_force(row, &ceiling, today, now) == Some(operation))
            .map(|(performer, row)| (performer, Grant::of_row(operation, row)))
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
            .map(|row| Grant::of_row(operation, row))
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

/// The facts of the parties stamped by transactions `since` did not see
/// (every party for `None`), read in one repeatable-read snapshot on `conn`.
async fn read_facts(
    conn: &mut sqlx::PgConnection,
    since: Option<&Snapshot>,
) -> Result<Changed, StorageError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    let changed = caller::store::load(&mut tx, since).await?;
    // NOTE: our own design (#5762), a read-only snapshot that writes nothing.
    tx.commit().await?;
    Ok(changed)
}

#[cfg(test)]
mod tests;
