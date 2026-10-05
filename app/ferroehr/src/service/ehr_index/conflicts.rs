// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Design-filled advisory duplicate-detection read.
//!
//! `master07 §Overview` names two error states the index metadata exists "to
//! detect and rectify": multiple EHRs recorded for one subject, and multiple
//! subjects recorded for one EHR. The SM defines no detection *operation* —
//! this read is our own design (advisory, never a hard reject: the N:M states
//! are legal-but-flagged per `resource_instance_type.adoc` `Duplicate`).
//!
//! Both scans run on the linkage pool, over the associations in force.

use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::ehr_index::types::{EhrIndexEntry, SubjectRef};
use crate::service::linkage::store;

use super::IndexError;

/// One detected index error state (master07 §Overview) — advisory only.
///
/// `pub`: the SM `I_EHR_INDEX` defines no detection operation (this read fills
/// that silence — our own design), so it has no SM trait binding and no
/// ITS-REST wire binding. It is exposed on the public [`FerroEhrService`]
/// surface as a native-API-only diagnostic; a route or admin-CLI binding would
/// be a spec-silent extension of our own, not a conformance requirement.
#[derive(Debug, Clone)]
pub enum IndexConflict {
    /// One subject is associated with more than one EHR (the
    /// "multiple EHRs … created in different locations" case). Carries every
    /// association of that subject so the operator can pick the `Primary`.
    SubjectWithMultipleEhrs {
        /// The subject the associations share, with its stored type.
        subject: SubjectRef,
        /// Every association of that subject, ordered by EHR id.
        entries: Vec<EhrIndexEntry>,
    },
    /// One EHR is associated with more than one subject (the
    /// "records merged … multiple subject ids" case).
    EhrWithMultipleSubjects {
        /// The EHR the associations share.
        ehr_id: EhrId,
        /// Every association of that EHR, ordered by subject key.
        entries: Vec<EhrIndexEntry>,
    },
}

impl FerroEhrService {
    /// Scan the index for the two master07 error states, returning every
    /// conflicting association group (empty = clean). Advisory: detection only,
    /// no mutation — rectification is the operator's `update_ehr_subject_status`
    /// / `remove_ehr_subject` call (I2/I4).
    ///
    /// # Errors
    /// [`IndexError::Service`] on a storage/database fault.
    pub async fn index_conflicts(&self) -> Result<Vec<IndexConflict>, IndexError> {
        let mut conflicts = Vec::new();

        // Subjects associated with more than one EHR.
        for row in &store::subjects_with_multiple_ehrs(&self.linkage_pool).await? {
            let subject_id: String = sqlx::Row::try_get(row, "subject_id")?;
            let namespace: String = sqlx::Row::try_get(row, "subject_namespace")?;
            let subject = SubjectRef::person(subject_id, namespace);
            let entries = self.index_subject_ehrs(&subject).await?;
            // Preserve the stored subject type (all associations share the key).
            let subject = entries.first().map_or(subject, |e| e.subject.clone());
            conflicts.push(IndexConflict::SubjectWithMultipleEhrs { subject, entries });
        }

        // EHRs associated with more than one subject.
        for ehr_id in store::ehrs_with_multiple_subjects(&self.linkage_pool).await? {
            let entries = self.index_ehr_subjects(ehr_id).await?;
            conflicts.push(IndexConflict::EhrWithMultipleSubjects { ehr_id, entries });
        }

        Ok(conflicts)
    }
}
