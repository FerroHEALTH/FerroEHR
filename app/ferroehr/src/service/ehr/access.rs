// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `EHR_ACCESS` — the EHR-wide access-control top-level structure
//! (arch-overview `master06-design_of_the_ehr.adoc` §`EHR_ACCESS`; RM ehr
//! `org.openehr.rm.ehr.ehr_access.adoc`). This file owns the object's default
//! (created under the EHR-creation CONTRIBUTION — there is no direct ITS-REST
//! `EHR_ACCESS` write), the settings read the protocol adapter consumes, and
//! this replica's copy of each EHR's settings; the commit validator lives in
//! [`validation`](super::validation), and the rows the copy is read from in
//! [`access_projection`](super::access_projection).
//!
//! The copy is read from the projection on a miss and kept until a
//! notification on [`CHANNEL`] names the EHR, which every commit that changes
//! its settings sends; while the listener is down every request reads the
//! projection, and a projection that cannot be read is `503`. The copy holds
//! at most [`EhrAccessCache`]'s capacity of EHRs, and beside them the
//! settings classes a query is judged over ([`crate::access_eval::class`]),
//! never every EHR that carries settings. No openEHR spec governs the copy:
//! our own design (#5588).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::access_eval::class::{Class, Classes, Keepers};
use crate::access_eval::condition::EhrEntry;
use crate::aql::sql::EhrAccessReaders;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::ehr::access_rule::SettingsCaller;
use crate::service::ehr::access_settings::{EhrAccessSettings, UnreadableSettings};
use moka::future::Cache;
use openehr_rm::prelude::{DvText, DvTextData, EhrAccess};
use serde_json::Value;
use sqlx::PgConnection;

use crate::policy_register::listen::{Dropping, Listened, Subscriber};
use crate::service::FerroEhrService;
use crate::storage::access_projection::{CHANNEL, NEW_CLASS, Stored, StoredSettings};
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{Change, commit_contribution};

impl FerroEhrService {
    /// Drop the copy of the `EHR_ACCESS` settings for `ehr_id`, called after
    /// any `EHR_ACCESS` commit so this replica's next access decision reflects
    /// the new version before the commit's notification arrives (the settings
    /// are change-controlled — RM ehr master04 §EHR Access).
    pub(crate) async fn invalidate_ehr_access(&self, ehr_id: EhrId) {
        self.ehr_access.forget(ehr_id).await;
    }

    /// The listener that drops this replica's copy of an EHR's settings when
    /// a commit changes them; [`Listened::spawn_listener`] starts it.
    #[must_use]
    pub fn ehr_access_listened(&self) -> &Arc<Listened> {
        &self.ehr_access_listened
    }

    /// Pre-warm the `EHR_ACCESS` cache for a just-created EHR as default-open
    /// (`None`). Every EHR is created with the settings-less
    /// `super::initial_ehr_access` (there is no direct `EHR_ACCESS` write —
    /// RM ehr master04 §EHR Access), so a fresh EHR is unconditionally
    /// default-open; seeding that entry saves the first-access DB miss. A
    /// later `EHR_ACCESS` commit evicts it through
    /// [`Self::invalidate_ehr_access`].
    pub(in crate::service) async fn prewarm_ehr_access_open(
        &self,
        ehr_id: EhrId,
        subject: Option<uuid::Uuid>,
    ) {
        if self.ehr_access_listened.is_listening() {
            let entry = EhrEntry {
                settings: Arc::new(None),
                subject,
                version: None,
            };
            self.ehr_access.insert(ehr_id, entry).await;
        }
    }

    /// Commit the default [`initial_ehr_access`] for an EHR that has none,
    /// inside the caller's transaction — the EHR-Extract import bootstrap
    /// ([`crate::service::message`]).
    ///
    /// `EHR.ehr_access` is 1..1 (RM ehr `ehr.adoc` invariant
    /// `Ehr_access_valid`) and RM ehr master04 §EHR Creation requires that
    /// creating an EHR yields "a root EHR object, an EHR Status object, and an
    /// EHR Access object … created and committed in a Contribution". A clone
    /// whose source extract carried no `EHR_ACCESS` would otherwise violate
    /// that invariant permanently and serve an `EHR` body without the mandatory
    /// reference, so the missing object is created locally.
    ///
    /// It is server-created content rather than replayed extract content, so it
    /// commits as a normal first `ORIGINAL_VERSION` (`249|creation|`,
    /// server-signed when signing is enabled) under its own CONTRIBUTION; the
    /// import CONTRIBUTION records the local act of committal for received
    /// originals, which "are never modified" (RM common master06 §Copying). Both
    /// CONTRIBUTIONs commit in the caller's single transaction, so the created
    /// EHR is complete atomically.
    ///
    /// # Errors
    /// The [`crate::versioning::change::commit_contribution`] write errors.
    pub(in crate::service) async fn commit_default_ehr_access(
        &self,
        tx: &mut PgConnection,
        ehr_id: EhrId,
        description: &str,
    ) -> Result<(), ServiceError> {
        let audit = self.audit(change_type::CREATION, description);
        commit_contribution(
            tx,
            Some(ehr_id),
            None,
            &audit,
            vec![(audit.clone(), initial_ehr_access_change())],
            Vec::new(),
            &self.signing_ctx(),
        )
        .await?;
        Ok(())
    }

    /// Commits the default `EHR_ACCESS`, with no settings, for every EHR that
    /// holds none, each in its own transaction, and returns how many it
    /// restored.
    ///
    /// `EHR.ehr_access` is 1..1 (RM ehr `ehr.adoc` invariant
    /// `Ehr_access_valid`); an EHR is left without one only when a migration
    /// drops settings of a scheme the server no longer keeps, so a boot finds
    /// none on every other occasion.
    ///
    /// # Errors
    /// The storage read errors and the commit's write errors.
    pub async fn restore_default_ehr_access(&self) -> Result<usize, ServiceError> {
        let missing = crate::storage::version_repo::ehr_access::ehrs_without_ehr_access(&self.pool)
            .await
            .map_err(ServiceError::from)?;
        for ehr_id in &missing {
            let mut conn = crate::db::acquire::acquire(&self.pool).await?;
            let mut tx = sqlx::Connection::begin(&mut *conn).await?;
            self.commit_default_ehr_access(&mut tx, *ehr_id, "EHR_ACCESS restored at boot")
                .await?;
            tx.commit().await?;
            self.invalidate_ehr_access(*ehr_id).await;
        }
        if !missing.is_empty() {
            tracing::info!(restored = missing.len(), "default EHR_ACCESS restored");
        }
        Ok(missing.len())
    }

    /// Read the EHR's projected `EHR_ACCESS` settings and its subject (the
    /// miss path). The settings are `None` when the EHR's `EHR_ACCESS`
    /// carries none, which the server-wide default governs.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the projection cannot be read, and
    /// as `entry_of`.
    async fn load_ehr_access(&self, ehr_id: EhrId) -> Result<EhrEntry, ServiceError> {
        let stored = crate::storage::access_projection::settings_of(&self.pool, ehr_id)
            .await
            .map_err(|e| ServiceError::overloaded(UNANSWERED, e))?;
        entry_of(ehr_id, stored)
    }

    /// The entries of `ehr_ids` from this replica's copy, the ones it does not
    /// hold read in one statement and copied: the EHRs a query served, whose
    /// access record names the grounds a direct read names (#5581).
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the projection cannot be read, and
    /// an internal fault naming an EHR whose settings do not read as the
    /// `access_directives` scheme.
    pub async fn ehr_access_entries(
        &self,
        ehr_ids: &[EhrId],
    ) -> Result<Vec<(EhrId, EhrEntry)>, ServiceError> {
        let listening = self.ehr_access_listened.is_listening();
        let mut entries = Vec::with_capacity(ehr_ids.len());
        let mut missing = Vec::new();
        for ehr_id in ehr_ids {
            match self.ehr_access.held(*ehr_id).await.filter(|_| listening) {
                Some(entry) => entries.push((*ehr_id, entry)),
                None => missing.push(*ehr_id),
            }
        }
        if missing.is_empty() {
            return Ok(entries);
        }
        let seen = self.ehr_access.drops();
        let read = crate::storage::access_projection::settings_of_many(&self.pool, &missing)
            .await
            .map_err(|e| ServiceError::overloaded(UNANSWERED, e))?;
        for (ehr_id, stored) in read {
            let entry = entry_of(ehr_id, stored)?;
            if listening {
                self.ehr_access.keep(ehr_id, entry.clone(), seen).await;
            }
            entries.push((ehr_id, entry));
        }
        Ok(entries)
    }

    /// The EHR's current `EHR_ACCESS` settings and its subject, from this
    /// replica's copy: what the access evaluator reads of one EHR. While the
    /// listener is down the projection is read on every call. No openEHR
    /// spec governs this copy: our own design.
    ///
    /// # Errors
    /// [`ServiceError`] when the miss path fails; the error is shared across
    /// concurrent callers of the single-flight load.
    pub async fn ehr_access_entry(&self, ehr_id: EhrId) -> Result<EhrEntry, ServiceError> {
        if !self.ehr_access_listened.is_listening() {
            return self.load_ehr_access(ehr_id).await;
        }
        // Clone the (cheap, Arc-backed) service into an owned, `'static` load
        // future so `moka`'s single-flight `try_get_with` can drive it.
        let svc = self.clone();
        let loaded = self
            .ehr_access
            .get_or_load(ehr_id, async move { svc.load_ehr_access(ehr_id).await })
            .await;
        owned_or_reload(loaded, || async { self.load_ehr_access(ehr_id).await }).await
    }

    /// The EHR's current `EHR_ACCESS` scheme settings, from this replica's
    /// copy, shared as the copy's own [`std::sync::Arc`] (the per-request gate
    /// never clones an access list). `None` when the EHR carries no settings,
    /// which the server-wide default governs.
    ///
    /// # Errors
    /// As [`Self::ehr_access_entry`].
    pub async fn current_ehr_access_settings(
        &self,
        ehr_id: EhrId,
    ) -> Result<Arc<Option<EhrAccessSettings>>, ServiceError> {
        Ok(self.ehr_access_entry(ehr_id).await?.settings)
    }

    /// The settings classes the store holds: the input of the condition a
    /// query applies to every EHR it could reach, where the path routes
    /// decide one EHR at a time ([`Self::current_ehr_access_settings`]). An
    /// EHR with no settings is of no class and falls to the server-wide
    /// default; an EHR of a class this copy does not hold is withheld.
    ///
    /// Copied as one entry, dropped by the notification of a new class, and
    /// read on every call while the listener is down. No openEHR spec
    /// governs the classes: our own design (#5588).
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the projection cannot be read.
    pub async fn ehr_access_classes(&self) -> Result<Arc<Classes>, ServiceError> {
        if !self.ehr_access_listened.is_listening() {
            return self.load_classes().await.map(Arc::new);
        }
        let svc = self.clone();
        let loaded = self
            .ehr_access
            .classes_or_load(async move { svc.load_classes().await })
            .await;
        owned_or_reload(loaded, || async { self.load_classes().await.map(Arc::new) }).await
    }

    /// Refuses a caller who may not commit the next `EHR_ACCESS` version of
    /// `ehr_id`, judged against the current settings read on the commit's
    /// own transaction ([`SettingsCaller::may_write`]).
    ///
    /// # Errors
    /// [`ServiceError::Forbidden`] (`403`) for a caller who is neither an
    /// administrator nor a gate-keeper of the current settings, and the
    /// storage error of the read.
    pub(in crate::service) async fn check_ehr_access_writer(
        &self,
        tx: &mut PgConnection,
        ehr_id: EhrId,
    ) -> Result<(), ServiceError> {
        let caller = self.settings_caller();
        if caller.administrator() {
            return Ok(());
        }
        let body =
            crate::storage::version_repo::ehr_access::current_ehr_access_body(tx, ehr_id).await?;
        let current = match body {
            Some(body) => {
                let value: Value = serde_json::from_str(&body)
                    .map_err(|e| ServiceError::internal("read a stored EHR_ACCESS body", e))?;
                EhrAccessSettings::held_or_read(&value)
            }
            None => None,
        };
        caller.may_write(current.as_ref())
    }

    /// Who the running request's caller is to the `EHR_ACCESS` settings.
    #[must_use]
    pub fn settings_caller(&self) -> SettingsCaller {
        SettingsCaller::current(&self.binding)
    }

    /// Whether the running request's caller may read the `EHR_ACCESS`
    /// settings of `ehr_id` ([`SettingsCaller::may_read`]).
    ///
    /// # Errors
    /// [`ServiceError`] when the settings cannot be read.
    pub async fn may_read_ehr_access(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        let caller = self.settings_caller();
        if caller.reads_every() {
            return Ok(true);
        }
        let settings = self.current_ehr_access_settings(ehr_id).await?;
        Ok(caller.may_read((*settings).as_ref()))
    }

    /// The EHRs whose `EHR_ACCESS` objects the running request's query reads:
    /// every EHR's for a caller who reads them all, otherwise the EHRs whose
    /// settings name the caller a gate-keeper, judged per class.
    ///
    /// # Errors
    /// [`ServiceError`] when the classes cannot be read.
    pub async fn ehr_access_readers(&self) -> Result<EhrAccessReaders, ServiceError> {
        let caller = self.settings_caller();
        if caller.reads_every() {
            return Ok(EhrAccessReaders::Every);
        }
        let classes = self.ehr_access_classes().await?;
        Ok(EhrAccessReaders::Only(Arc::new(Keepers::of(
            &classes, &caller,
        ))))
    }

    /// The miss path of [`Self::ehr_access_classes`]: every class the store
    /// holds. A class that does not read is left out, so every EHR of it is
    /// withheld.
    async fn load_classes(&self) -> Result<Classes, ServiceError> {
        let stored = crate::storage::access_projection::every_class(&self.pool)
            .await
            .map_err(|e| ServiceError::overloaded(UNANSWERED, e))?;
        let classes = stored
            .into_iter()
            .filter_map(|(id, text)| match Class::read(&text) {
                Ok(class) => Some((id, class)),
                Err(error) => {
                    let cause: &(dyn std::error::Error + 'static) = &error;
                    tracing::error!(
                        class = id,
                        error = cause,
                        "a settings class does not read; no query reaches its EHRs"
                    );
                    None
                }
            })
            .collect();
        Ok(Classes { classes })
    }
}

/// The entry of `ehr_id` as `stored` reads it.
///
/// # Errors
/// An internal fault naming the EHR when its settings do not read as the
/// `access_directives` scheme: a privacy control that cannot be read refuses
/// the EHR.
fn entry_of(ehr_id: EhrId, stored: Stored) -> Result<EhrEntry, ServiceError> {
    let unreadable = |error: Option<UnreadableSettings>| {
        tracing::error!(%ehr_id, "EHR_ACCESS settings do not parse; the EHR is refused");
        let message =
            format!("the EHR_ACCESS settings of EHR {ehr_id} do not parse, so the EHR is refused");
        match error {
            Some(error) => ServiceError::internal(message, error),
            None => ServiceError::internal(message, HeldClosed),
        }
    };
    let subject = subject_of(stored.subject.as_deref());
    let settings = match stored.settings {
        StoredSettings::None => None,
        StoredSettings::HeldClosed => return Err(unreadable(None)),
        StoredSettings::Settings(settings) => Some(
            EhrAccessSettings::from_projected(&settings)
                .map_err(|error| unreadable(Some(error)))?,
        ),
    };
    Ok(EhrEntry {
        version: settings.as_ref().and(stored.version),
        settings: Arc::new(settings),
        subject,
    })
}

/// The `PERSON` container a stored `ehr.subject_id` names.
fn subject_of(stored: Option<&str>) -> Option<uuid::Uuid> {
    // NOTE: the local source names each party by its container's UUID, so a subject
    // id that is no UUID names no PERSON of this store and has no care relationship.
    stored.and_then(|subject| subject.parse::<uuid::Uuid>().ok())
}

/// The message of the `503` of a projection that cannot be read.
const UNANSWERED: &str = "the EHR access settings could not be read";

/// The cause of the refusal of an EHR the projection holds closed.
#[derive(Debug, thiserror::Error)]
#[error("the stored EHR_ACCESS settings do not read as the access_directives scheme")]
struct HeldClosed;

/// Builds the `EHR_ACCESS` committed with every new EHR (RM ehr master04 §EHR
/// Creation).
///
/// `EHR_ACCESS` is a LOCATABLE with only the
/// optional `settings`; with no access-control scheme configured (Stage 1 has
/// no RBAC), it is committed with none.
///
/// `archetype_details` is mandatory: `EHR_ACCESS` carries the same
/// unconditional `Is_archetype_root` invariant as `EHR_STATUS`
/// (RM ehr `ehr_access.adoc`), so `Archetyped_valid` (RM common
/// `locatable.adoc`) requires the `ARCHETYPED` block on the root.
pub(in crate::service) fn initial_ehr_access_change() -> Change {
    Change::Create {
        allocated: None,
        kind: Kind::EhrAccess,
        canonical: initial_ehr_access(),
        template_id: None,
        signature: None,
        lifecycle_state: None,
        attestations: Vec::new(),
        other_input_version_uids: Vec::new(),
    }
}

/// The server-created first `EHR_ACCESS` body of an EHR (see
/// [`initial_ehr_access_change`]).
pub(in crate::service) fn initial_ehr_access() -> Value {
    const ARCHETYPE: &str = "openEHR-EHR-EHR_ACCESS.generic.v1";
    let access = EhrAccess {
        name: DvText::DvText(DvTextData {
            value: "EHR Access".to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: None,
            language: None,
            encoding: None,
        }),
        archetype_node_id: ARCHETYPE.to_owned(),
        uid: None,
        links: None,
        archetype_details: Some(crate::service::ehr::service::archetyped(ARCHETYPE)),
        feeder_audit: None,
        // `EHR_ACCESS.settings` (0..1) — see the doc comment above: with no
        // access-control scheme in force the default carries none.
        settings: None,
    };
    openehr_its::json::to_canonical_value(&access)
}

/// A shared per-EHR copy of the current `EHR_ACCESS` scheme settings.
///
/// The `EHR_ACCESS` gateway clause ("All access decisions to data in the EHR
/// must be made in accordance with the policies and rules in this object" —
/// RM ehr `ehr_access.adoc`) is consulted on **every** EHR-scoped request, so
/// the current settings are copied rather than read per request. An entry is
/// dropped by this replica's own `EHR_ACCESS` commit and by every notification
/// on [`CHANNEL`] that names its EHR; a notification of another payload, and a
/// listener that connects or loses its connection, drop every entry. A load
/// that a drop overtakes keeps nothing, so a load racing a commit never leaves
/// the settings before it in force.
///
/// No openEHR spec governs this copy — our own design/extension.
#[derive(Debug)]
pub(crate) struct EhrAccessCache {
    inner: Cache<EhrId, EhrEntry>,
    classes: Cache<(), Arc<Classes>>,
    /// Moved by every drop, so a load that started before one keeps nothing.
    drops: AtomicU64,
}

impl EhrAccessCache {
    /// A copy holding up to `capacity` EHRs' settings.
    fn new(capacity: u64) -> Self {
        Self {
            inner: Cache::builder().max_capacity(capacity).build(),
            classes: Cache::builder().max_capacity(1).build(),
            drops: AtomicU64::new(0),
        }
    }

    /// The copied entry of `ehr_id`, if this replica holds one.
    pub(in crate::service) async fn held(&self, ehr_id: EhrId) -> Option<EhrEntry> {
        self.inner.get(&ehr_id).await
    }

    /// How many drops this copy has seen, read before a load so
    /// [`Self::keep`] keeps nothing a drop overtook.
    pub(in crate::service) fn drops(&self) -> u64 {
        self.drops.load(Ordering::Acquire)
    }

    /// Copy `entry` for `ehr_id`, read after `seen` drops, unless a drop
    /// overtook the read.
    pub(in crate::service) async fn keep(&self, ehr_id: EhrId, entry: EhrEntry, seen: u64) {
        self.inner.insert(ehr_id, entry).await;
        if self.drops() != seen {
            self.inner.invalidate(&ehr_id).await;
        }
    }

    /// The copied settings for `ehr_id`, or load them via `init` (run at most
    /// once per key under contention) and copy the result unless a drop
    /// overtook the load.
    ///
    /// # Errors
    /// Propagates the `init` error (shared across concurrent callers as an
    /// `Arc<ServiceError>`).
    pub(in crate::service) async fn get_or_load<Fut>(
        &self,
        ehr_id: EhrId,
        init: Fut,
    ) -> Result<EhrEntry, Arc<ServiceError>>
    where
        Fut: Future<Output = Result<EhrEntry, ServiceError>>,
    {
        let seen = self.drops.load(Ordering::Acquire);
        let loaded = self.inner.try_get_with(ehr_id, init).await;
        if self.drops.load(Ordering::Acquire) != seen {
            self.inner.invalidate(&ehr_id).await;
        }
        loaded
    }

    /// Seed the copy with a value for `ehr_id` without a DB read — the
    /// pre-warm path of a freshly-created EHR, which is default-open. Any
    /// later `EHR_ACCESS` commit drops this entry via [`Self::forget`].
    pub(in crate::service) async fn insert(&self, ehr_id: EhrId, entry: EhrEntry) {
        self.inner.insert(ehr_id, entry).await;
    }

    /// Drop the copied settings for `ehr_id`.
    pub(in crate::service) async fn forget(&self, ehr_id: EhrId) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        self.inner.invalidate(&ehr_id).await;
    }

    /// Drop the copied classes.
    pub(in crate::service) fn forget_classes(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        self.classes.invalidate_all();
    }

    /// Drop every copied entry and the classes.
    pub(in crate::service) fn forget_every(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        self.inner.invalidate_all();
        self.classes.invalidate_all();
    }

    /// The copied classes, or load them via `init` (run at most once under
    /// contention) and copy the result unless a drop overtook the load.
    ///
    /// # Errors
    /// Propagates the `init` error (shared across concurrent callers as an
    /// `Arc<ServiceError>`).
    pub(in crate::service) async fn classes_or_load<Fut>(
        &self,
        init: Fut,
    ) -> Result<Arc<Classes>, Arc<ServiceError>>
    where
        Fut: Future<Output = Result<Classes, ServiceError>>,
    {
        let seen = self.drops();
        let loaded = self
            .classes
            .try_get_with((), async move { init.await.map(Arc::new) })
            .await;
        if self.drops() != seen {
            self.classes.invalidate(&()).await;
        }
        loaded
    }
}

impl Subscriber for EhrAccessCache {
    fn notified<'a>(&'a self, payload: &'a str) -> Dropping<'a> {
        Box::pin(async move {
            // NOTE: our own design, a payload that names no EHR and no new class, the
            // refill's, drops every EHR and every class.
            match payload.parse::<uuid::Uuid>() {
                Ok(ehr_id) => self.forget(EhrId(ehr_id)).await,
                Err(_) if payload == NEW_CLASS => self.forget_classes(),
                Err(_) => self.forget_every(),
            }
        })
    }

    fn reset(&self) -> Dropping<'_> {
        self.forget_every();
        Box::pin(std::future::ready(()))
    }
}

/// The listener of `pool` on [`CHANNEL`], dropping from `cache`.
pub(in crate::service) fn listened(pool: sqlx::PgPool, cache: &Arc<EhrAccessCache>) -> Listened {
    let cache: Arc<EhrAccessCache> = Arc::clone(cache);
    Listened::with_subscriber(pool, CHANNEL, cache)
}

/// The outcome of a single-flight load, with an error of the caller's own.
///
/// The caller that ran a failed load holds its error alone and gets it as it
/// was raised. A concurrent waiter only shares it, and a `ServiceError` cannot
/// be cloned, so the waiter reads again and answers with its own error of the
/// same class and body. No openEHR spec governs the cache: our own design.
async fn owned_or_reload<T, Fut>(
    loaded: Result<T, Arc<ServiceError>>,
    reload: impl FnOnce() -> Fut,
) -> Result<T, ServiceError>
where
    Fut: Future<Output = Result<T, ServiceError>>,
{
    match loaded {
        Ok(value) => Ok(value),
        Err(shared) => match Arc::try_unwrap(shared) {
            Ok(error) => Err(error),
            Err(_) => reload().await,
        },
    }
}

impl Default for EhrAccessCache {
    fn default() -> Self {
        Self::new(4096)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::refusal::{Missing, Refusal};

    /// The refusal a missing EHR's settings load answers.
    fn missing() -> ServiceError {
        ServiceError::not_found(Missing::Ehr, "no EHR with this id")
    }

    /// Two concurrent loads of one EHR's settings that fail answer the same
    /// refusal: the caller that ran the load keeps its error, and the waiter
    /// that only shared it reads again for its own.
    #[tokio::test]
    async fn concurrent_loads_of_a_missing_ehr_answer_the_same_refusal() {
        let cache = EhrAccessCache::new(16);
        let ehr_id = EhrId(uuid::Uuid::now_v7());
        let load = || async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            Err(missing())
        };
        let (first, second) = tokio::join!(
            cache.get_or_load(ehr_id, load()),
            cache.get_or_load(ehr_id, load())
        );
        let (first, second) = tokio::join!(
            owned_or_reload(first, || async { Err(missing()) }),
            owned_or_reload(second, || async { Err(missing()) }),
        );
        for answer in [first, second] {
            assert!(
                matches!(
                    &answer,
                    Err(ServiceError::NotFound(Refusal {
                        kind: Missing::Ehr,
                        message,
                        ..
                    })) if message == "no EHR with this id"
                ),
                "every concurrent caller answers the loader's refusal: {answer:?}"
            );
        }
    }
}
