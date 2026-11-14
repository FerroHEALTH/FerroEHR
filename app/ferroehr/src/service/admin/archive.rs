// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Archival storage movement (`archive_ehrs` / `archive_parties`): selected
//! EHRs, or selected parties with their relationships, move to archival
//! storage.
//!
//! NOTE: no openEHR spec governs archiving or storage tiering: the tier and
//! this movement are our own design.
//!
//! Each call moves the named objects' `version` rows to the cold partition —
//! one `UPDATE` of the partition key, with the `node` and `vo_attestation`
//! rows carried across by their foreign keys
//! ([`crate::storage::version_repo::tier`]) — and stamps the head row, in one
//! transaction. The move is reversible
//! ([`FerroEhrService::restore_archived_ehrs`],
//! [`FerroEhrService::restore_archived_parties`]) and invisible on the wire:
//! an object-addressed read names the parent relation and therefore reaches
//! either tier in one statement, and a write thaws the object first, so a
//! versioned object is never split across tiers. An unknown id aborts the
//! transaction before anything is moved. The AQL engine queries the hot tier
//! alone, so archived content leaves the queryable store until it is
//! restored.

use sqlx::Connection as _;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::storage::version_repo::tier;
use crate::system_log::event::EventActionCode;

impl FerroEhrService {
    /// Move every versioned object of each EHR to the cold archival tier
    /// (idempotent), all-or-nothing: every EHR is existence-checked before
    /// anything is written.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — any EHR is unknown
    ///   (`ehr_id_does_not_exist`); nothing is archived.
    /// - `exception` — a database fault mid-transaction (rolled back).
    /// - `service_overloaded` — an access record was refused under
    ///   `[audit] fail_mode = "closed"`.
    pub async fn archive_ehrs(&self, ehr_ids: &[EhrId]) -> Result<(), ServiceError> {
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        require_ehrs_exist(&mut tx, ehr_ids).await?;
        for &ehr_id in ehr_ids {
            let vo_ids = tier::hot_objects(&mut tx, ehr_id).await?;
            tier::freeze(&mut tx, &vo_ids, Some("archive_ehrs")).await?;
        }
        tx.commit().await?;
        self.emit_archive_access(ehr_ids, "archive")
    }

    /// Move each party's versioned object to the cold archival tier
    /// (idempotent), all-or-nothing: every id is checked to name a demographic
    /// PARTY root before anything is written.
    ///
    /// NOTE: no specification governs archiving: our own design; only the
    /// party VO is moved, and the related
    /// `PARTY_RELATIONSHIP`s stay independently addressable versioned objects a
    /// caller archives in their own right.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — any id names no
    ///   demographic PARTY root (`party_id_does_not_exist`); nothing is
    ///   archived.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn archive_parties(&self, party_ids: &[VoId]) -> Result<(), ServiceError> {
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;
        let mut live: Vec<VoId> = Vec::new();
        for &party_id in party_ids {
            let kind = tier::demographic_kind(&mut tx, party_id).await?;
            if !kind.as_deref().is_some_and(super::is_party_kind) {
                return Err(ServiceError::not_found(
                    Missing::Party,
                    format!("party {party_id}"),
                ));
            }
            live.push(party_id);
        }
        tier::freeze(&mut tx, &live, Some("archive_parties")).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Bring every archived versioned object of each EHR back from the cold
    /// tier to the primary tier, dropping its archive marker — the exact
    /// reverse of [`Self::archive_ehrs`], and idempotent in the same way (an
    /// EHR with nothing archived restores nothing and succeeds).
    ///
    /// NOTE: no specification governs archiving: this operation and its admin
    /// route (`POST /admin/archive/ehrs/restore`) are our own extension.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — any EHR is unknown.
    /// - `exception` — a database fault mid-transaction (rolled back).
    /// - `service_overloaded` — an access record was refused under
    ///   `[audit] fail_mode = "closed"`.
    pub async fn restore_archived_ehrs(&self, ehr_ids: &[EhrId]) -> Result<(), ServiceError> {
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        require_ehrs_exist(&mut tx, ehr_ids).await?;
        for &ehr_id in ehr_ids {
            let vo_ids = tier::cold_objects(&mut tx, ehr_id).await?;
            tier::thaw(&mut tx, &vo_ids).await?;
        }
        tx.commit().await?;
        self.emit_archive_access(ehr_ids, "archive-restore")
    }

    /// Record the move of each EHR between the tiers as an access to it, one
    /// record per EHR, so its access log shows who archived or restored it.
    ///
    /// # Errors
    /// `service_overloaded` when a record was refused under `[audit] fail_mode
    /// = "closed"`; the move itself is already committed.
    fn emit_archive_access(&self, ehr_ids: &[EhrId], register: &str) -> Result<(), ServiceError> {
        for &ehr_id in ehr_ids {
            self.emit_mark_access(EventActionCode::Update, ehr_id, None, register)?;
        }
        Ok(())
    }

    /// Bring each archived party's versioned object back from the cold tier to
    /// the primary tier, dropping its archive marker, all-or-nothing: the exact
    /// reverse of [`Self::archive_parties`].
    ///
    /// Spec-silent like its EHR twin above; served at
    /// `POST /admin/archive/parties/restore`, our own extension.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — any id names no
    ///   demographic PARTY root.
    /// - `exception` — a database fault mid-transaction (rolled back).
    pub async fn restore_archived_parties(&self, party_ids: &[VoId]) -> Result<(), ServiceError> {
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;
        let mut ids: Vec<VoId> = Vec::new();
        for &party_id in party_ids {
            let kind = tier::demographic_kind(&mut tx, party_id).await?;
            if !kind.as_deref().is_some_and(super::is_party_kind) {
                return Err(ServiceError::not_found(
                    Missing::Party,
                    format!("party {party_id}"),
                ));
            }
            ids.push(party_id);
        }
        tier::thaw(&mut tx, &ids).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Refuse the whole operation unless every named EHR exists.
///
/// Archive and restore are both all-or-nothing over a list, so the check runs
/// before either writes anything. One implementation for both: a divergence
/// here would let one of the two half-apply on an id the other rejects.
async fn require_ehrs_exist(
    tx: &mut sqlx::PgConnection,
    ehr_ids: &[EhrId],
) -> Result<(), ServiceError> {
    for &ehr_id in ehr_ids {
        crate::storage::ehr_repo::require_ehr(&mut *tx, ehr_id).await?;
    }
    Ok(())
}
