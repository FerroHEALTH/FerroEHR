// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a load checks of the `EHR_ACCESS` versions an archive carries before
//! it writes anything: settings the CONTRIBUTION write path would refuse, and
//! directives naming a party neither the CDR nor the archive holds (#5517).
//! No openEHR spec governs the archive: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use std::path::Path;

use serde_json::Value;

use super::{
    ArchiveReader, DemographicRecord, EhrRecord, Manifest, archive_refusal,
    resolve_version_documents,
};
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::admin::types::ExportFormat;
use crate::service::ehr::access_parties::Landing;

impl FerroEhrService {
    /// Refuses an archive whose `EHR_ACCESS` settings break the scheme
    /// ([`refuse_unreadable_access_settings`]) or whose directives name a
    /// party that neither the demographic store nor the archive's
    /// demographic segments hold ([`Self::check_directive_parties`]).
    ///
    /// # Errors
    /// Those refusals, each naming the archived EHR, and the
    /// [`ServiceError::BadRequest`] of a segment that is absent or does not
    /// parse.
    pub(super) async fn check_archived_access(
        &self,
        archive: &mut ArchiveReader,
        dir: &Path,
        manifest: &Manifest,
        format: ExportFormat,
    ) -> Result<(), ServiceError> {
        let settings = refuse_unreadable_access_settings(archive, dir, &manifest.segments, format)?;
        let landing = archived_parties(archive, dir, &manifest.demographic_segments)?;
        for (ehr_id, settings) in &settings {
            self.check_directive_parties(settings, &landing)
                .await
                .map_err(|e| for_archived_ehr(e, *ehr_id))?;
        }
        Ok(())
    }
}

/// Refuse an archive any of whose `EHR_ACCESS` versions carries settings the
/// CONTRIBUTION write path would refuse
/// ([`check_written_settings`](crate::service::ehr::access_settings::check_written_settings)),
/// naming the setting and its EHR, and return every settings it read with
/// its EHR. A canonical-XML payload that cannot be read is left to the
/// per-record report the load gives it.
///
/// # Errors
/// The refusal of `check_written_settings`, prefixed with the EHR, and the
/// [`ServiceError::BadRequest`] of a segment that is absent or does not parse.
fn refuse_unreadable_access_settings(
    archive: &mut ArchiveReader,
    dir: &Path,
    segments: &[String],
    format: ExportFormat,
) -> Result<Vec<(EhrId, Value)>, ServiceError> {
    let mut read = Vec::new();
    for segment in segments {
        let bytes = archive.read(segment)?;
        let records: Vec<EhrRecord> =
            serde_json::from_slice(&bytes).map_err(|e| archive_refusal(dir, segment, e))?;
        for mut record in records {
            record.versions.retain(|v| v.kind == "EHR_ACCESS");
            if format == ExportFormat::OpenehrCanonicalXml
                && resolve_version_documents(archive, &mut record).is_err()
            {
                continue;
            }
            let ehr_id = record.ehr.id;
            for version in &record.versions {
                let Some(settings) = version.body.get("settings").filter(|s| !s.is_null()) else {
                    continue;
                };
                crate::service::ehr::access_settings::check_written_settings(settings)
                    .map_err(|e| for_archived_ehr(e, ehr_id))?;
                read.push((ehr_id, settings.clone()));
            }
        }
    }
    Ok(read)
}

/// The parties the archive's demographic segments carry, by container and RM
/// type, which the load lands after its EHRs.
///
/// # Errors
/// The [`ServiceError::BadRequest`] of a segment that is absent or does not parse.
fn archived_parties(
    archive: &mut ArchiveReader,
    dir: &Path,
    segments: &[String],
) -> Result<Landing, ServiceError> {
    let mut landing = Landing::new();
    for segment in segments {
        let bytes = archive.read(segment)?;
        let records: Vec<DemographicRecord> =
            serde_json::from_slice(&bytes).map_err(|e| archive_refusal(dir, segment, e))?;
        landing.extend(
            records
                .into_iter()
                .map(|record| (record.vo_id, record.kind)),
        );
    }
    Ok(landing)
}

/// Name the archived EHR a settings refusal is about.
fn for_archived_ehr(error: ServiceError, ehr_id: EhrId) -> ServiceError {
    match error {
        ServiceError::BadRequest(mut refusal) => {
            refusal.message = format!("archived EHR {ehr_id}: {}", refusal.message);
            ServiceError::BadRequest(refusal)
        }
        ServiceError::Unprocessable { kind, violation } => ServiceError::Unprocessable {
            kind,
            violation: violation.with_path(format!("archived EHR {ehr_id}")),
        },
        other => other,
    }
}
