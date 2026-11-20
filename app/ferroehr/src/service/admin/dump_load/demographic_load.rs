// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The load of an archive's demographic wave: its shared rows, then each
//! ehr-less container in a transaction of its own, with the side-table rows
//! rebuilt from the loaded head and each care relationship judged against
//! the access policy in force (#6123). No openEHR spec governs the archive:
//! our own design.

use super::{
    ArchiveReader, DemographicCommons, DemographicRecord, load_versions, refusals,
    resolve_demographic_documents,
};
use crate::error::ServiceError;
use crate::party_binding::store::reindex_loaded;
use crate::service::FerroEhrService;
use crate::service::admin::types::{DumpLoadFailReport, ExportFormat};
use crate::storage::dump_repo;
use crate::storage::dump_repo::write::StoredAudit;

impl FerroEhrService {
    /// Loads one archived demographic record, returning the per-entity failure
    /// report where the record is skipped rather than loaded.
    ///
    /// A duplicate container id, an unreadable canonical-XML payload and a
    /// care relationship the access policy's bounds refuse are per-entity
    /// reports, never fatal — the same shape as duplicate EHR ids.
    ///
    /// # Errors
    /// The storage errors of the existence check and the load itself.
    pub(super) async fn load_demographic_record(
        &self,
        archive: &mut ArchiveReader,
        mut record: DemographicRecord,
        format: ExportFormat,
    ) -> Result<Option<DumpLoadFailReport>, ServiceError> {
        let record_id = record.vo_id;
        let report = |kind: &str, error: String| {
            Ok(Some(DumpLoadFailReport {
                entity_type: kind.to_owned(),
                entity_id: record_id.to_string(),
                dump_status: false,
                error: Some(error),
            }))
        };
        if dump_repo::read::demographic_container_exists(&self.demographic_pool, record.vo_id)
            .await?
        {
            let error = "a demographic container with this id already exists";
            return report(&record.kind, error.to_owned());
        }
        if format == ExportFormat::OpenehrCanonicalXml
            && let Err(message) = resolve_demographic_documents(archive, &mut record)
        {
            return report(
                &record.kind,
                format!("the archive's canonical-XML version payload is unreadable: {message}"),
            );
        }
        let kind = record.kind.clone();
        match self.load_one_demographic(record).await {
            Ok(()) => Ok(None),
            // A care relationship the access policy's bounds refuse (#6123): that one record
            // is reported and rolled back, the rest of the archive loads.
            Err(ServiceError::ValidationFailed(violations)) => report(&kind, refusals(&violations)),
            Err(error) => Err(ServiceError::flattened(error)),
        }
    }

    /// Load the demographic wave's shared rows. Audits and contributions may
    /// already exist on a partial load into a non-empty repository (one
    /// demographic contribution can commit several containers), so both
    /// inserts are identity-preserving no-ops on an existing id.
    pub(super) async fn load_demographic_commons(
        &self,
        commons: &DemographicCommons,
    ) -> Result<(), ServiceError> {
        let mut conn = self.recorded_conn(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;
        dump_repo::write::insert_audits(&mut tx, &commons.audits, StoredAudit::Keep).await?;
        dump_repo::write::insert_contributions(&mut tx, None, &commons.contributions).await?;
        self.commit_act_recorded(tx, Vec::new()).await?;
        Ok(())
    }

    /// Load one ehr-less demographic container: its per-version audits, every
    /// version (`version` + re-decomposed `node` rows, `ehr_id` NULL),
    /// attestations, demographic tags and archive rows — one transaction, so a
    /// failed container commits nothing.
    ///
    /// # Errors
    /// The storage errors of the load itself.
    async fn load_one_demographic(&self, record: DemographicRecord) -> Result<(), ServiceError> {
        let mut conn = self.recorded_conn(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;
        dump_repo::write::insert_audits(&mut tx, &record.audits, StoredAudit::Keep).await?;
        load_versions(&mut tx, &self.privacy, None, record.versions).await?;
        dump_repo::write::insert_attestations(&mut tx, &record.attestations).await?;
        dump_repo::write::insert_item_tags(&mut tx, None, &record.item_tags).await?;
        dump_repo::write::restore_archives(&mut tx, &record.archives).await?;
        // The archive carries no index row: each is rebuilt from the loaded head.
        let policy = self.access_policy.in_force();
        reindex_loaded(&mut tx, &self.binding, &policy, &record.kind, record.vo_id).await?;
        self.commit_act_recorded(tx, Vec::new()).await?;
        Ok(())
    }
}
