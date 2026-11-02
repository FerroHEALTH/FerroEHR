// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 3(3) delay of the person's own access: the policy, the
//! professional's hold and release, the register, and the expiry records.
//!
//! Regulation (EU) 2025/327 Art. 3(1) gives a person access to their data
//! "immediately after" it is registered, and Art. 3(3) lets a Member State
//! delay that access "for a limited period of time until a health
//! professional is able to properly communicate and explain to the natural
//! persons concerned information that can have a significant impact on their
//! health" (`docs/law/eu/ehds/text.html`; recital 10 gives the diagnosis of an
//! incurable disease as the example). The software part is a hold on one
//! composition that a patient-context read treats as absent until a
//! professional releases it or its limit passes; which data a Member State
//! delays, and for how long, is the deployer's configuration. Setting,
//! releasing and expiring a hold are each an access record. No openEHR spec
//! governs any of it: our own design/extension.

pub mod config;

use std::sync::Arc;
use std::time::Duration;

use sqlx::Connection as _;

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::admin::access_delay::config::AccessDelayPolicy;
use crate::service::error::ServiceError;
use crate::storage::access_delay;
use crate::system_log::event::EventActionCode;

/// The register name a hold and its release carry in the access record.
const REGISTER: &str = "access-delay";
/// The register name the expiry of a hold carries in the access record.
const EXPIRY: &str = "access-delay-expiry";

impl FerroEhrService {
    /// Install the `[access_delay]` policy; `None`, the default, leaves the
    /// person's access immediate (EHDS Art. 3(1)).
    #[must_use]
    pub fn with_access_delay(mut self, policy: Option<AccessDelayPolicy>) -> Self {
        self.access_delay = policy.map(Arc::new);
        self
    }

    /// Whether the deployment enables the EHDS Art. 3(3) delay.
    #[must_use]
    pub fn access_delay_enabled(&self) -> bool {
        self.access_delay.is_some()
    }

    /// Hold one composition from the person's own access until `until`, or for
    /// the configured limit, on a health professional's act.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — the delay is not enabled, an id or
    ///   `until` is malformed, `until` is not after now or past the configured
    ///   limit, or the object is not a COMPOSITION.
    /// - `ehr_id_does_not_exist` / `versioned_object_does_not_exist` (`404`) —
    ///   the EHR is unknown, or the object is not in it.
    /// - `exception` — a database fault (rolled back); `service_overloaded`
    ///   when the access record is refused under `[audit] fail_mode = "closed"`.
    pub async fn delay_person_access(
        &self,
        ehr_id: &str,
        vo_id: &str,
        until: Option<&str>,
        note: Option<&str>,
    ) -> Result<(), ServiceError> {
        // NOTE: EHDS Art. 3(3) is a Member State option; with `[access_delay]` off,
        // Art. 3(1) immediate access holds and no hold may be placed.
        let Some(policy) = self.access_delay.as_deref() else {
            return Err(ServiceError::precondition(
                "access_delay is not enabled on this server: EHDS Art. 3(3) delays the \
                 person's access only where a Member State rule provides for it",
            ));
        };
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        let vo_id = VoId(super::parse_uuid(vo_id, "versioned object")?);
        let until = until
            .map(|raw| {
                raw.parse::<jiff::Timestamp>().map_err(|e| {
                    ServiceError::precondition(format!(
                        "`until` {raw:?} is not an RFC 3339 instant: {e}"
                    ))
                })
            })
            .transpose()?;
        self.check_mark_target(ehr_id, Some(vo_id)).await?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        let held = access_delay::hold(&mut tx, (ehr_id, vo_id), until, policy.hours(), note)
            .await
            .map_err(ServiceError::Storage)?;
        if held.is_none() {
            return Err(ServiceError::precondition(format!(
                "nothing was held: {vo_id} is not a COMPOSITION, or `until` is not after now \
                 and within access_delay.hours ({}) of now (EHDS Art. 3(3): a limited period)",
                policy.hours()
            )));
        }
        tx.commit().await.map_err(ServiceError::Database)?;
        self.emit_mark_access(EventActionCode::Create, ehr_id, Some(vo_id), REGISTER)?;
        Ok(())
    }

    /// Release every hold of one composition still in force, so the person
    /// reads it: the health professional's act once the data has been
    /// explained (EHDS Art. 3(3)). Releasing nothing succeeds.
    ///
    /// # Errors
    /// The [`Self::delay_person_access`] rejections for the ids, and the same
    /// database and access-record faults.
    pub async fn release_person_access(
        &self,
        ehr_id: &str,
        vo_id: &str,
    ) -> Result<(), ServiceError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        let vo_id = VoId(super::parse_uuid(vo_id, "versioned object")?);
        self.check_mark_target(ehr_id, Some(vo_id)).await?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        access_delay::release(&mut tx, vo_id)
            .await
            .map_err(ServiceError::Storage)?;
        tx.commit().await.map_err(ServiceError::Database)?;
        self.emit_mark_access(EventActionCode::Delete, ehr_id, Some(vo_id), REGISTER)?;
        Ok(())
    }

    /// The access-delay register of one EHR, newest hold first.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — a malformed id.
    /// - `ehr_id_does_not_exist` (`404`) — the EHR is unknown.
    /// - `exception` — a database fault.
    pub async fn access_delay_register(
        &self,
        ehr_id: &str,
    ) -> Result<Vec<access_delay::AccessDelayRow>, ServiceError> {
        let ehr_id = EhrId(super::parse_uuid(ehr_id, "EHR")?);
        self.check_mark_target(ehr_id, None).await?;
        self.emit_mark_access(EventActionCode::Read, ehr_id, None, REGISTER)?;
        access_delay::register(&self.pool, ehr_id)
            .await
            .map_err(ServiceError::Storage)
    }

    /// Whether the versioned object `vo_id` is withheld from the person now;
    /// `false` for an unknown object.
    ///
    /// # Errors
    /// `exception` — a database fault.
    pub async fn is_object_withheld(&self, vo_id: VoId) -> Result<bool, ServiceError> {
        let mut conn = acquire(&self.pool).await?;
        access_delay::is_withheld(&mut conn, vo_id)
            .await
            .map_err(ServiceError::Storage)
    }

    /// Write the access record of every hold whose limit passed unreleased,
    /// once each; returns how many were recorded.
    ///
    /// The hold stops withholding at its limit whether or not this runs: the
    /// reads compare the limit with the clock. This is only the record.
    ///
    /// # Errors
    /// `exception` — a database fault; `service_overloaded` when an access
    /// record is refused under `[audit] fail_mode = "closed"`, and then no
    /// expiry is marked recorded.
    pub async fn record_access_delay_expiries(&self) -> Result<usize, ServiceError> {
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        let expired = access_delay::take_expired(&mut tx)
            .await
            .map_err(ServiceError::Storage)?;
        for hold in &expired {
            self.emit_mark_access(
                EventActionCode::Delete,
                hold.ehr_id,
                Some(hold.vo_id),
                EXPIRY,
            )?;
        }
        tx.commit().await.map_err(ServiceError::Database)?;
        Ok(expired.len())
    }

    /// The ITI-81 retrieval of one person's own access trail (EHDS Art. 9):
    /// a record about a composition held from them (Art. 3(3)) is left out
    /// until the hold is released or expires, and then served with its
    /// original time.
    ///
    /// # Errors
    /// `precondition_violation` when no local store is wired, or `exception`
    /// when a store query fails.
    #[cfg(feature = "fhir")]
    pub async fn person_audit_event_search(
        &self,
        filter: &crate::system_log::store::AuditSearchFilter,
    ) -> Result<(i64, Vec<serde_json::Value>), ServiceError> {
        let Some(audit_store) = &self.audit_store else {
            return self.audit_event_search(filter).await;
        };
        let fault = |e: crate::system_log::AuditError| {
            ServiceError::internal("search the audit record repository", e)
        };
        let ehrs = audit_store.ehrs_of(filter).await.map_err(fault)?;
        let now = jiff::Timestamp::now();
        match self.person_holds(&ehrs, now).await? {
            Some(holds) => audit_store.search_for_person(filter, &holds, now).await,
            None => audit_store.search(filter).await,
        }
        .map_err(fault)
    }

    /// The register of the holds on the EHRs `ehrs`, replayed oldest first,
    /// or `None` when none of them is in force at `now`, so nothing is
    /// withheld from the person's trail.
    #[cfg(feature = "fhir")]
    async fn person_holds(
        &self,
        ehrs: &[String],
        now: jiff::Timestamp,
    ) -> Result<Option<ehds_logging::withheld::Holds>, ServiceError> {
        // NOTE: an audit record names its EHR as text, and one that is no UUID names no EHR
        // of this store, so it is absent from the register, never an error.
        let ids: Vec<uuid::Uuid> = ehrs
            .iter()
            .filter_map(|ehr| uuid::Uuid::parse_str(ehr).ok())
            .collect();
        let rows = access_delay::holds_of(&self.pool, &ids)
            .await
            .map_err(ServiceError::Storage)?;
        if !rows
            .iter()
            .any(|row| row.released_at.is_none() && row.withheld_until > now)
        {
            return Ok(None);
        }
        let mut holds = ehds_logging::withheld::Holds::default();
        for row in &rows {
            // NOTE: EHDS Art. 3(3), the hold's own mark (`access-delay:<vo>`) would tell the
            // person a result exists, so it is held with the composition.
            for key in [row.vo_id.to_string(), format!("{REGISTER}:{}", row.vo_id)] {
                holds.hold(key.as_str(), row.withheld_until);
                if let Some(released) = row.released_at {
                    holds.release(&key, released).map_err(|e| {
                        crate::service::error::internal_fault_caused("replay the access delays", &e)
                    })?;
                }
            }
        }
        Ok(Some(holds))
    }

    /// Record the expiries every `every` in a detached task that ends with the
    /// runtime; nothing is spawned while the delay is off.
    pub fn spawn_access_delay_expiries(self: &Arc<Self>, every: Duration) {
        if self.access_delay_enabled() {
            tokio::spawn(record_expiries(Arc::clone(self), every));
        }
    }
}

/// The detached loop behind [`FerroEhrService::spawn_access_delay_expiries`].
#[expect(
    clippy::infinite_loop,
    reason = "a detached background task with no shutdown channel; it ends when the \
              runtime drops it, and `-> !` cannot be spawned on stable"
)]
async fn record_expiries(service: Arc<FerroEhrService>, every: Duration) {
    let mut ticks = tokio::time::interval(every);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        if let Err(error) = service.record_access_delay_expiries().await {
            tracing::warn!(%error, "recording the expired access delays failed; retried next tick");
        }
    }
}
