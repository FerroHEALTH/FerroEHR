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
//! notification on [`crate::storage::access_projection::CHANNEL`] names the EHR, which every commit that changes
//! its settings sends; while the listener is down every request reads the
//! projection, and a projection that cannot be read is `503`. The copy holds
//! at most [`super::access_cache::EhrAccessCache`]'s capacity of EHRs, and beside them the
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

use crate::access_eval::class::{Class, Classes, Keepers};
use crate::access_eval::condition::EhrEntry;
use crate::access_eval::stale::StaleCopies;
use crate::aql::sql::readers::EhrAccessReaders;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::ehr::access_cache::{EHR, Lookup, looked_up, owned_or_reload, refilled};
use crate::service::ehr::access_rule::SettingsCaller;
use crate::service::ehr::access_settings::{EhrAccessSettings, UnreadableSettings};
use crate::service::ehr::access_view::SettingsView;
use openehr_rm::prelude::{DvText, DvTextData, EhrAccess};
use serde_json::Value;
use sqlx::PgConnection;

use crate::policy_register::listen::Listened;
use crate::service::FerroEhrService;
use crate::storage::access_projection::{Revision, Stored, StoredSettings};
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::body::Body;
use crate::versioning::change::{Change, commit_contribution};

impl FerroEhrService {
    /// Drop the copy of the `EHR_ACCESS` settings and subject for `ehr_id`,
    /// called after any `EHR_ACCESS` commit and an `EHR_STATUS` commit that
    /// moved the subject, the two things the copy is revised by, so this replica's
    /// next access decision reflects the new version before the commit's
    /// notification arrives (the settings are change-controlled — RM ehr
    /// master04 §EHR Access).
    ///
    /// A request that judged the EHR before its own commit is judged again on
    /// the committed settings, so what it reads after the commit is decided
    /// by them. When they cannot be read the earlier verdict stays, and each
    /// read refuses it as judged on an older revision.
    pub(crate) async fn invalidate_ehr_access(&self, ehr_id: EhrId) {
        self.ehr_access.forget(ehr_id).await;
        let Some(access) = crate::access_eval::condition::current() else {
            return;
        };
        if access.judgement(ehr_id).is_none() {
            return;
        }
        let judged = match self.load_ehr_access(ehr_id).await {
            Ok(entry) => access
                .judge_ehr(ehr_id, &entry)
                .await
                .map_err(ServiceError::from),
            Err(error) => Err(error),
        };
        if let Err(error) = judged {
            let cause: &(dyn std::error::Error + 'static) = &error;
            tracing::warn!(
                %ehr_id,
                error = cause,
                "the committed EHR_ACCESS settings could not be read; the request's reads refuse"
            );
        }
    }

    /// The EHRs whose copy a read of the running request found stale, which
    /// the copy drops before its next read
    /// ([`crate::access_eval::stale::StaleCopies`]).
    #[must_use]
    pub fn stale_ehr_access_copies(&self) -> Arc<StaleCopies> {
        self.ehr_access.stale()
    }

    /// The listener that drops this replica's copy of an EHR's settings when
    /// a commit changes them; [`crate::policy_register::listen::Listeners::spawn`] starts it.
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
        subject: Option<crate::storage::access_projection::EhrSubject>,
    ) {
        if self.ehr_access_listened.is_listening() {
            let entry = EhrEntry {
                settings: Arc::new(None),
                subject: subject.clone(),
                version: None,
                revision: Revision {
                    settings: None,
                    subject,
                },
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
        description: &'static str,
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
    /// drops settings of a scheme the server does not keep, so a boot finds
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
            // NOTE: our own design (#5762), the boot-time restore, which runs outside any request.
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
        self.ehr_access.forget_stale().await;
        let listening = !self.ehr_access_listened.unlistened();
        let mut entries = Vec::with_capacity(ehr_ids.len());
        let mut missing = Vec::new();
        for ehr_id in ehr_ids {
            match self.ehr_access.entry(*ehr_id).await.filter(|_| listening) {
                Some(entry) => entries.push((*ehr_id, entry)),
                None => missing.push(*ehr_id),
            }
        }
        if listening {
            looked_up(EHR, Lookup::Hit, entries.len());
            looked_up(EHR, Lookup::Miss, missing.len());
        }
        if missing.is_empty() {
            return Ok(entries);
        }
        let seen = self.ehr_access.drops();
        let read = crate::storage::access_projection::settings_of_many(&self.pool, &missing).await;
        if listening {
            refilled(EHR, &read);
        }
        let read = read.map_err(|e| ServiceError::overloaded(UNANSWERED, e))?;
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
        self.ehr_access.forget_stale().await;
        if self.ehr_access_listened.unlistened() {
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
    /// default; a query reaching an EHR of a class this copy does not hold
    /// reads the classes again (`service::query::stale`, #5794).
    ///
    /// Copied as one entry, dropped by the notification of a new class, and
    /// read on every call while the listener is down. No openEHR spec
    /// governs the classes: our own design (#5588).
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the projection cannot be read.
    pub async fn ehr_access_classes(&self) -> Result<Arc<Classes>, ServiceError> {
        if self.ehr_access_listened.unlistened() {
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
    /// own transaction ([`SettingsCaller::may_write`]), or whose versions
    /// write a `marking` other than the current one: the marking is the
    /// controller's alone, and a version committed by anyone else carries it
    /// unchanged (draft access control, The Decision Procedure, Reading and
    /// Writing the Settings). The controller's current exclusions are carried
    /// into each `EHR_ACCESS` version of `changes` such a caller commits
    /// ([`crate::service::ehr::access_exclusions::carry`]).
    ///
    /// # Errors
    /// [`ServiceError::Forbidden`] (`403`) for a caller who is neither an
    /// administrator nor a gate-keeper of the current settings, or who
    /// changes the marking or the exclusions, and the storage error of the
    /// read.
    pub(in crate::service) async fn check_ehr_access_writer(
        &self,
        tx: &mut PgConnection,
        ehr_id: EhrId,
        changes: &mut [(crate::versioning::audit::AuditInput, Change)],
    ) -> Result<(), ServiceError> {
        let caller = self.settings_caller_on(ehr_id);
        if caller.administrator() {
            return Ok(());
        }
        let body =
            crate::storage::version_repo::ehr_access::current_ehr_access_body(tx, ehr_id).await?;
        let stored: Option<Value> = body
            .map(|body| serde_json::from_str(&body))
            .transpose()
            .map_err(|e| ServiceError::internal("read a stored EHR_ACCESS body", e))?;
        let current = stored.as_ref().and_then(EhrAccessSettings::held_or_read);
        caller.may_write(current.as_ref())?;
        let marking = stored
            .as_ref()
            .and_then(|value| value.get("settings"))
            .and_then(|settings| settings.get("marking"));
        if changes
            .iter()
            .filter(|(_, c)| c.kind() == Kind::EhrAccess)
            .any(|(_, c)| !same_marking(written_marking(c), marking))
        {
            return Err(ServiceError::not_permitted(
                "only the controller sets, changes or clears the EHR_ACCESS marking; a version \
                 committed by anyone else carries it unchanged",
            ));
        }
        let held = stored
            .as_ref()
            .and_then(crate::service::ehr::access_exclusions::held);
        crate::service::ehr::access_exclusions::carry(held, changes)
    }

    /// Who the running request's caller is to the `EHR_ACCESS` settings.
    #[must_use]
    pub fn settings_caller(&self) -> SettingsCaller {
        SettingsCaller::current(&self.binding)
    }

    /// Who the running request's caller is to the `EHR_ACCESS` settings of
    /// `ehr_id`: [`Self::settings_caller`], and the record's person as well
    /// when the request found that EHR's subject to be the `PERSON` the party
    /// binding names the caller (EHDS Art. 3; our own design, #6106).
    #[must_use]
    pub fn settings_caller_on(&self, ehr_id: EhrId) -> SettingsCaller {
        let caller = self.settings_caller();
        if crate::access_eval::condition::current()
            .is_some_and(|access| access.is_record(Some(ehr_id)))
        {
            return caller.into_subject();
        }
        caller
    }

    /// How the running request's caller reads the `EHR_ACCESS` settings of
    /// `ehr_id` ([`SettingsCaller::view`]).
    ///
    /// # Errors
    /// [`ServiceError`] when the settings cannot be read.
    pub async fn ehr_access_view(&self, ehr_id: EhrId) -> Result<SettingsView, ServiceError> {
        let caller = self.settings_caller_on(ehr_id);
        if caller.reads_every() {
            return Ok(SettingsView::Whole);
        }
        let settings = self.current_ehr_access_settings(ehr_id).await?;
        Ok(caller.view((*settings).as_ref()))
    }

    /// The EHRs whose `EHR_ACCESS` objects the running request's query reads:
    /// every EHR's for the controller, every EHR's whose settings hold no
    /// exclusions for the subject, otherwise the EHRs whose settings name the
    /// caller a gate-keeper and hold no exclusions, judged per class.
    ///
    /// # Errors
    /// [`ServiceError`] when the classes cannot be read.
    pub async fn ehr_access_readers(&self) -> Result<EhrAccessReaders, ServiceError> {
        let caller = self.settings_caller();
        if caller.administrator() {
            return Ok(EhrAccessReaders::Every);
        }
        if caller.reads_every() {
            return Ok(EhrAccessReaders::Unexcluded);
        }
        let classes = self.ehr_access_classes().await?;
        Ok(EhrAccessReaders::Only(Arc::new(Keepers::of(
            &classes, &caller,
        ))))
    }

    /// The EHRs whose `EHR_ACCESS` objects a query under `condition` reads.
    ///
    /// A query pinned to one EHR reads that EHR's alone, so how the caller
    /// reads its settings is decided once from this replica's copy
    /// ([`Self::ehr_access_view`]), the copy the pinned verdict was judged
    /// on, and the statement probes no settings row for it; any other query
    /// reads them as [`Self::ehr_access_readers`] says. No openEHR spec
    /// governs this: our own design (#5746).
    ///
    /// # Errors
    /// As [`Self::ehr_access_view`] and [`Self::ehr_access_readers`].
    pub async fn query_access_readers(
        &self,
        condition: &crate::access_eval::condition::AccessCondition,
    ) -> Result<EhrAccessReaders, ServiceError> {
        let Some(pinned) = condition.pinned() else {
            return self.ehr_access_readers().await;
        };
        let unexcluded = !self.settings_caller().administrator();
        Ok(match self.ehr_access_view(EhrId(pinned.ehr)).await? {
            SettingsView::Whole if unexcluded => EhrAccessReaders::Unexcluded,
            SettingsView::Whole => EhrAccessReaders::Every,
            SettingsView::WithoutMarking => EhrAccessReaders::UnexcludedWithoutMarking,
            SettingsView::Hidden => EhrAccessReaders::Only(Arc::new(Keepers {
                groups: Vec::new(),
                party: None,
            })),
        })
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
    let subject = stored.revision.subject.clone();
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
        revision: stored.revision,
    })
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
/// `archetype_details` is mandatory: `EHR_ACCESS` carries the same
/// unconditional `Is_archetype_root` invariant as `EHR_STATUS`
/// (RM ehr `ehr_access.adoc`), so `Archetyped_valid` (RM common
/// `locatable.adoc`) requires the `ARCHETYPED` block on the root.
pub(in crate::service) fn initial_ehr_access_change() -> Change {
    Change::Create {
        allocated: None,
        kind: Kind::EhrAccess,
        body: initial_ehr_access().into(),
        template_id: None,
        signature: None,
        lifecycle_state: None,
        attestations: Vec::new(),
        other_input_version_uids: Vec::new(),
    }
}

/// The server-created first `EHR_ACCESS` body of an EHR (see
/// [`initial_ehr_access_change`]).
pub(in crate::service) fn initial_ehr_access() -> EhrAccess {
    const ARCHETYPE: &str = "openEHR-EHR-EHR_ACCESS.generic.v1";
    EhrAccess {
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
        archetype_details: Some(Box::new(crate::service::ehr::service::archetyped(
            ARCHETYPE,
        ))),
        feeder_audit: None,
        // `EHR_ACCESS.settings` (0..1) — see the doc comment above: with no
        // access-control scheme in force the default carries none.
        settings: None,
    }
}

/// Whether two `settings.marking` values are the same marking: read as the
/// class, so a version that spells the stored marking with or without its
/// optional `_type` members carries it unchanged; a value that does not read
/// is compared as it is written.
fn same_marking(written: Option<&Value>, stored: Option<&Value>) -> bool {
    use openehr_rm::draft::access_control::access_control::confidentiality_marking::ConfidentialityMarking;
    // NOTE: our own design, a marking that does not read is no class to compare, so it is
    // compared as written, which refuses any change to it.
    let read = |value: &Value| {
        openehr_its::json::from_canonical_value::<ConfidentialityMarking>(value).ok()
    };
    match (written, stored) {
        (None, None) => true,
        (Some(written), Some(stored)) => match (read(written), read(stored)) {
            (Some(written), Some(stored)) => written == stored,
            _ => written == stored,
        },
        _ => false,
    }
}

/// The `settings.marking` an `EHR_ACCESS` change writes: `None` for a
/// deletion, or a version that carries no marking.
pub(in crate::service) fn written_marking(change: &Change) -> Option<&Value> {
    match change {
        Change::Create { body, .. } | Change::Modify { body, .. } => match body {
            Body::EhrAccess(rm) => rm
                .settings
                .as_ref()
                .and_then(|settings| settings.members().get("marking")),
            _ => body
                .canonical()
                .and_then(|view| view.get("settings"))
                .and_then(|settings| settings.get("marking")),
        },
        Change::Delete { .. } => None,
    }
}
