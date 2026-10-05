// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_EHR_INDEX` operations (I1–I5, `i_ehr_index.adoc`) + the two
//! design-filled reads.
//!
//! The associations live in `linkage.subject_ehr` and every statement over them
//! runs on the linkage pool through the [`crate::service::linkage`] store; only the
//! EHR-existence probe runs on the clinical pool. No openEHR spec governs the
//! storage — our own design; master07 governs the operation semantics and error
//! names.
//!
//! Every domain failure is an [`IndexError`], whose `From<IndexError> for
//! SmError` maps `ehr_id_does_not_exist` / `subject_id_does_not_exist` onto
//! their dedicated `CallStatusType` variants — never the generic
//! `versioned_object_does_not_exist` (`i_ehr_index.adoc §Errors`).

use uuid::Uuid;

use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::ehr_index::types::{EhrIndexEntry, LocationDesc, ResourceStatus, SubjectRef};
use crate::service::linkage::store;
use crate::service::status::SmError;
use crate::system_log::event::EventActionCode;

use super::{IndexError, require_association, row_to_entry, validate_status};

/// Parse an `ehr_id` UUID. An unparseable id is a `400` precondition failure;
/// a well-formed-but-unknown id surfaces as `ehr_id_does_not_exist` at the DB
/// check (`i_ehr_index.adoc §Errors`).
#[expect(
    clippy::map_err_ignore,
    reason = "the mapped error already names the resource and echoes the \
              rejected token; the discarded `uuid::Error` adds only its own \
              wording, which is not part of the wire contract"
)]
fn parse_ehr_id(raw: &str) -> Result<EhrId, SmError> {
    Uuid::parse_str(raw)
        .map(EhrId)
        .map_err(|_| SmError::precondition(format!("invalid ehr id: {raw}")))
}

/// How many entries a read served, as the access record's count.
///
/// `usize` is wider than `u64` on no target this builds for, so the fallback is
/// unreachable arithmetic rather than a described state; saturating keeps the
/// conversion non-panicking without a suppression.
fn entry_count(entries: &[EhrIndexEntry]) -> u64 {
    u64::try_from(entries.len()).unwrap_or(u64::MAX)
}

impl FerroEhrService {
    /// SM `add_ehr_subject` (I1): associate `subject` with `ehr_id` with an
    /// optional status + location. The EHR must exist.
    ///
    /// NOTE: "Add" is realized as an idempotent upsert over the association in
    /// force — re-adding the same subject refreshes its status/location rather
    /// than erroring; the `0..1` cardinality of `add_ehr_subject` permits this.
    /// Status defaults to a `Primary` instance (`i_ehr_index.adoc`).
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — `ehr_id` is not a well-formed UUID,
    ///   or a `start_valid_time`/`end_valid_time` is not an ISO date-time.
    /// - `ehr_id_does_not_exist` — no EHR with that id.
    /// - `exception` — a database fault while writing.
    pub async fn add_ehr_subject(
        &self,
        ehr_id: String,
        subject: SubjectRef,
        status: Option<ResourceStatus>,
        loc: Option<LocationDesc>,
    ) -> Result<(), SmError> {
        let ehr_id = parse_ehr_id(&ehr_id)?;
        self.index_ehr_exists(ehr_id).await?;
        let status = status.unwrap_or_default();
        validate_status(&status).map_err(IndexError::Service)?;
        let outcome =
            store::add_association(&self.linkage_pool, ehr_id, &subject, &status, loc.as_ref())
                .await
                .map_err(IndexError::from);
        self.record_index_access(
            EventActionCode::Create,
            ehr_id,
            &subject,
            1,
            outcome.is_ok(),
        )?;
        Ok(outcome?)
    }

    /// SM `update_ehr_subject_status` (I2): update the status of an existing
    /// (`ehr_id`, `subject`) association.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — `ehr_id` is not a well-formed UUID,
    ///   or a `start_valid_time`/`end_valid_time` is not an ISO date-time.
    /// - `ehr_id_does_not_exist` — no EHR with that id.
    /// - `subject_id_does_not_exist` — the subject is not associated with the
    ///   EHR (the update matched no row).
    /// - `exception` — a database fault while writing.
    pub async fn update_ehr_subject_status(
        &self,
        ehr_id: String,
        subject: SubjectRef,
        status: ResourceStatus,
    ) -> Result<(), SmError> {
        let ehr_id = parse_ehr_id(&ehr_id)?;
        self.index_ehr_exists(ehr_id).await?;
        validate_status(&status).map_err(IndexError::Service)?;
        let updated = store::set_association_status(&self.linkage_pool, ehr_id, &subject, &status)
            .await
            .map_err(IndexError::from)?;
        self.record_index_access(
            EventActionCode::Update,
            ehr_id,
            &subject,
            updated,
            updated > 0,
        )?;
        Ok(require_association(updated, &subject)?)
    }

    /// SM `update_ehr_subject_loc_desc` (I3): update (or clear, `loc = None`)
    /// the location descriptor of an existing association.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — `ehr_id` is not a well-formed UUID.
    /// - `ehr_id_does_not_exist` — no EHR with that id.
    /// - `subject_id_does_not_exist` — the subject is not associated with the
    ///   EHR (the update matched no row).
    /// - `exception` — a database fault while writing.
    pub async fn update_ehr_subject_loc_desc(
        &self,
        ehr_id: String,
        subject: SubjectRef,
        loc: Option<LocationDesc>,
    ) -> Result<(), SmError> {
        let ehr_id = parse_ehr_id(&ehr_id)?;
        self.index_ehr_exists(ehr_id).await?;
        let updated =
            store::set_association_location(&self.linkage_pool, ehr_id, &subject, loc.as_ref())
                .await
                .map_err(IndexError::from)?;
        self.record_index_access(
            EventActionCode::Update,
            ehr_id,
            &subject,
            updated,
            updated > 0,
        )?;
        Ok(require_association(updated, &subject)?)
    }

    /// SM `remove_ehr_subject` (I4): end the `subject`↔`ehr_id` association
    /// (the subject may remain associated with other EHRs).
    ///
    /// NOTE: the association is CLOSED, not deleted — the linkage domain
    /// corrects forward and its role holds no `DELETE`; a closed period is
    /// exactly "this association ended", and no read serves it afterwards.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — `ehr_id` is not a well-formed UUID.
    /// - `ehr_id_does_not_exist` — no EHR with that id.
    /// - `subject_id_does_not_exist` — the subject is not associated with the
    ///   EHR (the close matched no row).
    /// - `exception` — a database fault while writing.
    pub async fn remove_ehr_subject(
        &self,
        ehr_id: String,
        subject: SubjectRef,
    ) -> Result<(), SmError> {
        let ehr_id = parse_ehr_id(&ehr_id)?;
        self.index_ehr_exists(ehr_id).await?;
        let closed = store::close_association(&self.linkage_pool, ehr_id, &subject)
            .await
            .map_err(IndexError::from)?;
        self.record_index_access(
            EventActionCode::Delete,
            ehr_id,
            &subject,
            closed,
            closed > 0,
        )?;
        Ok(require_association(closed, &subject)?)
    }

    /// SM `remove_subject` (I5): end all associations for `subject`.
    ///
    /// # Errors
    /// - `subject_id_does_not_exist` — the subject has no associations (the
    ///   close matched no row).
    /// - `exception` — a database fault while writing.
    pub async fn remove_subject(&self, subject: SubjectRef) -> Result<(), SmError> {
        let closed = store::close_subject_associations(&self.linkage_pool, &subject)
            .await
            .map_err(IndexError::from)?;
        self.record_subject_access(EventActionCode::Delete, &subject, closed, closed > 0)?;
        Ok(require_association(closed, &subject)?)
    }

    /// The subjects associated with an EHR (design-filled read; the SM defines
    /// no read operations — our own design). Empty for an unknown EHR.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — `ehr_id` is not a well-formed UUID.
    /// - `exception` — a database fault while reading.
    pub async fn ehr_subjects(&self, ehr_id: String) -> Result<Vec<EhrIndexEntry>, SmError> {
        let ehr_id = parse_ehr_id(&ehr_id)?;
        let entries = self.index_ehr_subjects(ehr_id).await;
        let count = entries.as_ref().map_or(0, |found| entry_count(found));
        self.record_ehr_access(EventActionCode::Read, ehr_id, count, entries.is_ok())?;
        Ok(entries?)
    }

    /// The EHRs associated with a subject (design-filled read; the SM defines
    /// no read operations — our own design). Empty for an unknown subject.
    ///
    /// # Errors
    /// - `exception` — a database fault while reading.
    pub async fn subject_ehrs(&self, subject: SubjectRef) -> Result<Vec<EhrIndexEntry>, SmError> {
        let entries = self.index_subject_ehrs(&subject).await;
        let count = entries.as_ref().map_or(0, |found| entry_count(found));
        self.record_subject_access(EventActionCode::Read, &subject, count, entries.is_ok())?;
        Ok(entries?)
    }

    /// Confirm an EHR exists ([`IndexError::EhrDoesNotExist`] →
    /// `ehr_id_does_not_exist` otherwise). This distinguishes an unknown EHR
    /// from an unknown association to the caller (`master07 §Errors`).
    ///
    /// The one statement of these operations that runs on the CLINICAL pool:
    /// the EHR's existence is a clinical fact, and the linkage role can see no
    /// clinical relation by design.
    async fn index_ehr_exists(&self, ehr_id: EhrId) -> Result<(), IndexError> {
        let found: Option<Uuid> = sqlx::query_scalar("SELECT id FROM ehr WHERE id = $1")
            .bind(ehr_id)
            .fetch_optional(&self.pool)
            .await?;
        found.map(|_| ()).ok_or(IndexError::EhrDoesNotExist(ehr_id))
    }

    /// The associations of one EHR, as [`EhrIndexEntry`]s — shared by
    /// [`Self::ehr_subjects`] and the duplicate-detection scan
    /// ([`super::conflicts`]).
    pub(super) async fn index_ehr_subjects(
        &self,
        ehr_id: EhrId,
    ) -> Result<Vec<EhrIndexEntry>, IndexError> {
        let rows = store::ehr_associations(&self.linkage_pool, ehr_id).await?;
        Ok(rows.iter().map(row_to_entry).collect::<Result<_, _>>()?)
    }

    /// The EHRs associated with a subject, as [`EhrIndexEntry`]s — shared by
    /// [`Self::subject_ehrs`] and the duplicate-detection scan
    /// ([`super::conflicts`]).
    pub(super) async fn index_subject_ehrs(
        &self,
        subject: &SubjectRef,
    ) -> Result<Vec<EhrIndexEntry>, IndexError> {
        let rows = store::subject_associations(&self.linkage_pool, subject).await?;
        Ok(rows.iter().map(row_to_entry).collect::<Result<_, _>>()?)
    }
}
