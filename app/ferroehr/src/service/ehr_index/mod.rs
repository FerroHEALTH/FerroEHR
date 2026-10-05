// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR Index service (`service/ehr_index/`) — SM `I_EHR_INDEX`.
//!
//! Holds N:M subject↔EHR associations with duplicate-management metadata
//! (`docs/specs/openehr/SM/docs/openehr_platform/master07-ehr_index_service.adoc`
//! and the UML class `i_ehr_index.adoc`).
//!
//! Layout: `index` = the SM write operations (I1–I5) + the design-filled
//! reads, each public method parsing its `ehr_id` at the boundary;
//! `conflicts` = the design-filled advisory duplicate-detection read;
//! [`types`] = the SM information structures (`RESOURCE_STATUS`,
//! `RESOURCE_INSTANCE_TYPE`, `LOCATION_DESC`, the `OBJECT_REF` subject key).
//!
//! NOTE: the SM defines no versioning for the index, so entries are plain
//! writes emitting no CONTRIBUTION or version; no openEHR spec governs the
//! storage mechanism (our own design) while master07 governs the operation
//! semantics and error names.
//!
//! The associations live in `linkage.subject_ehr`, reached through the linkage
//! pool by the store the [`crate::service::linkage`] module owns, because
//! master07 §Overview says
//! what the index is for: "In a privacy-supporting environment, this enables
//! EHRs to be persisted with only an EHR id; the EHR Index has to be used to
//! obtain the subject identifier". Only the EHR-existence probe runs on the
//! clinical pool, so each operation crosses at the APPLICATION layer over two
//! pools and no statement names both domains.
//!
//! The index and the `ehr.subject_id` promotion are decoupled: an EHR created
//! through the normal API is not auto-indexed here, and the index models the
//! full N:M state master07 §Overview requires ("There is no limit on the number
//! of subject identifiers associated with a given EHR id, and vice versa"). No
//! wire is mounted, EHR Index having no ITS-REST contract.

pub mod conflicts;
pub(crate) mod index;
pub mod types;

use sqlx::Row;

use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::ehr_index::types::{EhrIndexEntry, LocationDesc, ResourceStatus, SubjectRef};
use crate::service::error::ServiceError;
use crate::service::linkage::LinkageError;
use crate::service::status::{CallStatusType, SmError};
use crate::system_log::event::EventActionCode;

/// The precise EHR-index failure kind.
///
/// `master07 §Errors` declares distinct `ehr_id_does_not_exist` and
/// `subject_id_does_not_exist` statuses, which must NOT collapse to the generic
/// `versioned_object_does_not_exist` (`i_ehr_index.adoc §Errors`).
/// [`From<IndexError> for SmError`] maps each variant onto its dedicated
/// [`CallStatusType`]; a generic [`ServiceError`] (a DB/codec fault) rides
/// through unchanged.
// Nominal `pub`: `FerroEhrService::index_conflicts` (a `pub` method — the
// design-filled detection read has no SM trait binding) carries this type in
// its public signature.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// `ehr_id_does_not_exist` — the addressed EHR is unknown.
    #[error("EHR {0} does not exist (ehr_id_does_not_exist)")]
    EhrDoesNotExist(EhrId),
    /// `subject_id_does_not_exist` — no such subject / association.
    #[error("subject {}@{} is not associated (subject_id_does_not_exist)", .0.id, .0.namespace)]
    SubjectDoesNotExist(SubjectRef),
    /// A generic service/DB fault (mapped by the shared table).
    #[error(transparent)]
    Service(#[from] ServiceError),
}

impl From<sqlx::Error> for IndexError {
    fn from(e: sqlx::Error) -> Self {
        IndexError::Service(ServiceError::from(e))
    }
}

impl From<IndexError> for SmError {
    /// Map the precise EHR-index errors onto their dedicated SM statuses
    /// (`master07 §Errors`), bypassing the generic
    /// `NotFound → versioned_object_does_not_exist` collapse the shared
    /// [`From<ServiceError> for SmError`] applies.
    fn from(e: IndexError) -> Self {
        match e {
            IndexError::EhrDoesNotExist(id) => SmError::new(
                CallStatusType::EhrIdDoesNotExist,
                format!("EHR {id} does not exist"),
            ),
            IndexError::SubjectDoesNotExist(subject) => SmError::new(
                CallStatusType::SubjectIdDoesNotExist,
                format!(
                    "subject {}@{} is not associated",
                    subject.id, subject.namespace
                ),
            ),
            IndexError::Service(e) => e.into(),
        }
    }
}

/// Refuse a `RESOURCE_STATUS` whose validity bounds are not ISO-8601
/// date-times. An unparseable value is a `400`.
///
/// The bounds are stored verbatim inside the association's `status` document,
/// so this is the only place their form is enforced; a value that reached
/// storage unchecked would come back out of a read as a token no client can
/// interpret.
///
/// NOTE: `RESOURCE_STATUS.start_valid_time`/`end_valid_time` are typed
/// `@@` (an unresolved placeholder) in the SM — a recorded spec defect
/// (`resource_status.adoc:20,24`); implemented as ISO date-time strings.
#[expect(
    clippy::map_err_ignore,
    reason = "the mapped error already echoes the rejected token; the discarded \
              parse error adds only its own wording, which is not part of the \
              wire contract"
)]
fn validate_status(status: &ResourceStatus) -> Result<(), ServiceError> {
    for raw in [
        status.start_valid_time.as_deref(),
        status.end_valid_time.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        raw.parse::<jiff::Timestamp>()
            .map_err(|_| ServiceError::precondition(format!("invalid valid_time: {raw}")))?;
    }
    Ok(())
}

impl FerroEhrService {
    /// Record one EHR Index operation as a linkage-domain access, naming both
    /// halves of the association it touched.
    ///
    /// The relation these operations read and write is the one the linkage
    /// service owns, so a crossing here is recorded exactly as a crossing
    /// there is ([`crate::service::linkage`]). The record carries the
    /// namespace, never the identifier value: a trail holding the subject id
    /// would hold the very data the separation keeps out of readable storage.
    ///
    /// # Errors
    /// [`SmError`] when the sender rejected the record under
    /// `fail_mode = "closed"`; the caller withholds its result.
    fn record_index_access(
        &self,
        action: EventActionCode,
        ehr_id: EhrId,
        subject: &SubjectRef,
        count: u64,
        succeeded: bool,
    ) -> Result<(), SmError> {
        self.emit_linkage_access(
            action,
            format!("ehr-subject:{ehr_id}@{}", subject.namespace),
            Some(ehr_id),
            count,
            succeeded,
        )
        .map_err(index_access_refused)
    }

    /// Record one subject-scoped EHR Index operation, which names no single
    /// EHR.
    ///
    /// # Errors
    /// [`SmError`] when the sender rejected the record under
    /// `fail_mode = "closed"`.
    fn record_subject_access(
        &self,
        action: EventActionCode,
        subject: &SubjectRef,
        count: u64,
        succeeded: bool,
    ) -> Result<(), SmError> {
        self.emit_linkage_access(
            action,
            format!("subject:{}", subject.namespace),
            None,
            count,
            succeeded,
        )
        .map_err(index_access_refused)
    }

    /// Record one EHR-scoped EHR Index read.
    ///
    /// # Errors
    /// [`SmError`] when the sender rejected the record under
    /// `fail_mode = "closed"`.
    fn record_ehr_access(
        &self,
        action: EventActionCode,
        ehr_id: EhrId,
        count: u64,
        succeeded: bool,
    ) -> Result<(), SmError> {
        self.emit_linkage_access(
            action,
            format!("ehr-subject:{ehr_id}"),
            Some(ehr_id),
            count,
            succeeded,
        )
        .map_err(index_access_refused)
    }
}

/// Render a refused access record as the server fault it is.
///
/// A completed operation whose crossing could not be recorded is withheld under
/// `fail_mode = "closed"`, and the caller is told nothing about the audit
/// pipeline beyond that the server failed.
fn index_access_refused(error: LinkageError) -> SmError {
    SmError::exception("the linkage access record could not be taken").with_source(error)
}

/// Map a zero-rows-affected write to [`IndexError::SubjectDoesNotExist`]
/// (`subject_id_does_not_exist`).
fn require_association(affected: u64, subject: &SubjectRef) -> Result<(), IndexError> {
    if affected == 0 {
        return Err(IndexError::SubjectDoesNotExist(subject.clone()));
    }
    Ok(())
}

/// Reassemble one [`EhrIndexEntry`] from a `linkage.subject_ehr` row.
///
/// A stored document that no longer decodes as the designed contract is a
/// server fault: it surfaces through the `?` (the DB error path), never as a
/// blanked field.
fn row_to_entry(row: &sqlx::postgres::PgRow) -> Result<EhrIndexEntry, sqlx::Error> {
    let ehr_id: EhrId = row.try_get("ehr_id")?;
    let subject = SubjectRef {
        id: row.try_get("subject_id")?,
        namespace: row.try_get("subject_namespace")?,
        r#type: row
            .try_get::<Option<String>, _>("subject_type")?
            .unwrap_or_else(|| SubjectRef::DEFAULT_TYPE.to_owned()),
    };
    let status = row
        .try_get::<Option<sqlx::types::Json<ResourceStatus>>, _>("status")?
        .map(|j| j.0)
        .unwrap_or_default();
    let location = row
        .try_get::<Option<sqlx::types::Json<LocationDesc>>, _>("location")?
        .map(|j| j.0);
    Ok(EhrIndexEntry {
        ehr_id: ehr_id.to_string(),
        subject,
        status,
        location,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two declared EHR-index errors map to their dedicated SM statuses
    /// (`master07 §Errors`), never the generic
    /// `versioned_object_does_not_exist`.
    #[test]
    fn index_errors_map_to_dedicated_statuses() {
        let ehr = EhrId::minted(&crate::licence::stamp::StampKey::fail_safe());
        let ehr_sm: SmError = IndexError::EhrDoesNotExist(ehr).into();
        assert_eq!(ehr_sm.status, CallStatusType::EhrIdDoesNotExist);

        let subject = SubjectRef::person("p1", "demographic");
        let subj_sm: SmError = IndexError::SubjectDoesNotExist(subject).into();
        assert_eq!(subj_sm.status, CallStatusType::SubjectIdDoesNotExist);

        // A generic service miss still routes through the shared table (404),
        // carrying the status it was constructed with.
        let svc: SmError = IndexError::Service(ServiceError::sm(
            CallStatusType::VersionedObjectDoesNotExist,
            "x",
        ))
        .into();
        assert_eq!(svc.status, CallStatusType::VersionedObjectDoesNotExist);
    }

    /// A status whose validity bounds are absent or ISO-8601 is accepted; one
    /// carrying a token that is neither is a `400`, so the stored document
    /// cannot hold a bound no reader can interpret.
    #[test]
    fn valid_time_parsing() {
        assert!(validate_status(&ResourceStatus::default()).is_ok());
        assert!(
            validate_status(&ResourceStatus {
                start_valid_time: Some("2021-01-01T00:00:00Z".to_owned()),
                ..ResourceStatus::default()
            })
            .is_ok()
        );
        assert!(
            validate_status(&ResourceStatus {
                end_valid_time: Some("not-a-time".to_owned()),
                ..ResourceStatus::default()
            })
            .is_err()
        );
    }
}
