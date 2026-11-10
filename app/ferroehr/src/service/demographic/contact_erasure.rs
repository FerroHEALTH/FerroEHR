// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The erasure of a deleted SCIM User's contact values.
//!
//! A SCIM `DELETE` logically deletes the `PERSON` and keeps every version for
//! the record-keeping duties that need the professional's identification
//! (GDPR Art. 6(1)(c)). Only that attribution minimum waits for the national
//! retention horizon: the name, the binding key and the identifiers. The
//! contacts serve no attribution, so `[auth.scim] contact_erasure_minutes` after
//! the deletion the `contacts` of every version is removed (GDPR Art.
//! 5(1)(e), "no longer than is necessary"), at the deletion itself when the
//! period is zero.
//!
//! Each version's body loses its `contacts` attribute, which RM demographic
//! `PARTY` holds as an optional list (`org.openehr.rm.demographic.party.adoc`,
//! invariant `Contacts_valid`: Void or non-empty), so the erased body is a
//! valid `PERSON`. The node rows are then re-derived from the new body by the
//! node rebuild, which checks that the rows reassemble it. The version's
//! signature was taken over the content the erasure removed part of, so it is
//! voided: keeping it would fail every verification, and signing again would
//! claim an act its committer never made (RM common
//! `master06-change_control_package.adoc` §Digital Signature). The version
//! keeps its uid, audit and contribution. No openEHR spec governs which values
//! of a deleted party are erased: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments, edited \
              in place so every other attribute keeps its stored bytes and order"
)]

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use sqlx::Connection as _;
use sqlx::PgConnection;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::ids::VoId;
use crate::party_binding::provisioned;
use crate::service::FerroEhrService;
use crate::service::admin::integrity::rebuild::{NodeRebuildOutcome, rebuild_in_tx};
use crate::storage::erasure;
use crate::storage::version_repo::tier;
use crate::system_log::event::EventActionCode;
use crate::versioning::change::canonical_body_text;
use crate::versioning::change::resolve::content_digest;

/// The most deleted Users one sweep erases.
const SWEEP_BATCH: i64 = 100;

impl FerroEhrService {
    /// Spawns the sweep that erases, every `every`, the contacts of the
    /// deleted Users of `binding_issuers` once `delay_minutes` have passed;
    /// nothing when no client is declared.
    pub fn spawn_contact_erasure(
        self: &Arc<Self>,
        binding_issuers: Vec<String>,
        delay_minutes: u32,
        every: Duration,
    ) {
        if !binding_issuers.is_empty() {
            tokio::spawn(sweep(
                Arc::clone(self),
                binding_issuers,
                delay_minutes,
                every,
            ));
        }
    }

    /// Erases the contacts of the deleted Users of the clients of
    /// `binding_issuers` whose deletion is at least `delay_minutes` old,
    /// at most one batch per call, each in its own transaction; returns how
    /// many it erased.
    ///
    /// # Errors
    /// A storage failure, a stored body that does not parse or does not
    /// decompose once erased, or an access record the audit trail refuses;
    /// the User in progress is rolled back and the next sweep retries it.
    pub async fn erase_due_contacts(
        &self,
        binding_issuers: &[String],
        delay_minutes: u32,
    ) -> Result<usize, ServiceError> {
        if binding_issuers.is_empty() {
            return Ok(0);
        }
        let mut conn = acquire(&self.demographic_pool).await?;
        let due = provisioned::contacts_due(&mut conn, binding_issuers, delay_minutes, SWEEP_BATCH)
            .await?;
        let mut erased = 0;
        for party in due {
            let mut tx = conn.begin().await?;
            if self.erase_contacts_in(&mut tx, party).await? {
                tx.commit().await?;
                erased += 1;
            }
        }
        Ok(erased)
    }

    /// Erases the contacts of every version of the `PERSON` `party` on `tx`
    /// and records the erasure; `false` when they were already erased or the
    /// party is gone.
    ///
    /// # Errors
    /// As [`Self::erase_due_contacts`].
    pub(in crate::service) async fn erase_contacts_in(
        &self,
        tx: &mut PgConnection,
        party: VoId,
    ) -> Result<bool, ServiceError> {
        if !provisioned::lock_contact_erasure(tx, party).await? {
            return Ok(false);
        }
        // NOTE: a write always thaws first (`crate::storage::version_repo::tier`), so an
        // archived PERSON is erased in the primary tier and frozen again with its marker.
        let archive_reason = tier::lock_cold_marker(tx, party).await?;
        if archive_reason.is_some() {
            tier::thaw(tx, &[party]).await?;
        }
        for (sys_version, body) in erasure::locked_version_bodies(tx, party).await? {
            let Some(text) = body else {
                continue;
            };
            let mut value: Value = serde_json::from_str(&text)
                .map_err(|e| ServiceError::internal("parse a stored PERSON body", e))?;
            let removed = value
                .as_object_mut()
                .and_then(|root| root.shift_remove("contacts"));
            if removed.is_none() {
                continue;
            }
            let erased = canonical_body_text(&value)?;
            let digest = content_digest(&value)?;
            erasure::replace_version_body(tx, party, sys_version, &erased, &digest).await?;
            if let NodeRebuildOutcome::Refused { reason } =
                rebuild_in_tx(tx, party, sys_version).await?
            {
                return Err(ServiceError::exception(format!(
                    "the contacts of PERSON {party} version {sys_version} could not be erased: \
                     {reason}"
                )));
            }
        }
        provisioned::mark_contacts_erased(tx, party).await?;
        if let Some(reason) = archive_reason {
            tier::freeze(tx, &[party], reason.as_deref()).await?;
        }
        self.record_party_access(
            EventActionCode::Delete,
            format!("party:{}#contacts", party.0),
        )?;
        Ok(true)
    }
}

/// The detached loop behind [`FerroEhrService::spawn_contact_erasure`].
#[expect(
    clippy::infinite_loop,
    reason = "a detached background task with no shutdown channel; it ends when the \
              runtime drops it, and `-> !` cannot be spawned on stable"
)]
async fn sweep(
    service: Arc<FerroEhrService>,
    binding_issuers: Vec<String>,
    delay_minutes: u32,
    every: Duration,
) {
    let mut ticks = tokio::time::interval(every);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        match service
            .erase_due_contacts(&binding_issuers, delay_minutes)
            .await
        {
            Ok(0) => {}
            Ok(erased) => tracing::info!(erased, "erased the contacts of deleted SCIM Users"),
            Err(error) => tracing::warn!(
                error = %crate::error::ErrorChain::new(&error),
                "erasing the contacts of deleted SCIM Users failed; retried next tick"
            ),
        }
    }
}
