// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a load checks of the episodes an archive's compositions are placed
//! in before it writes anything: every one is the `episode` of a care
//! relationship of its EHR's subject, in the demographic store or the
//! archive's demographic segments, which the load lands after its EHRs
//! (draft access control package, Episodes of Care: declared episodes). No
//! openEHR spec governs the archive: our own design (#6412).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use uuid::Uuid;

use super::archived_access::for_archived_ehr;
use super::{
    ArchiveReader, DemographicRecord, EhrRecord, Manifest, archive_refusal,
    resolve_demographic_documents, resolve_version_documents,
};
use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::party_binding::grants::caller::{FactValue, store};
use crate::party_binding::grants::care::care_of;
use crate::party_binding::grants::derive::GrantFacts;
use crate::service::FerroEhrService;
use crate::service::admin::types::ExportFormat;
use crate::storage::access_projection::EhrSubject;
use crate::storage::dump_repo::VersionRecord;
use crate::versioning::Kind;
use crate::versioning::episode::{self, EpisodeKey};

/// An archived EHR whose compositions are placed: its subject and the
/// episodes they name.
type Placed = (EhrId, Option<EhrSubject>, BTreeSet<EpisodeKey>);

impl FerroEhrService {
    /// Refuses an archive one of whose EHRs holds a COMPOSITION placed in an
    /// episode no care relationship of the EHR's subject names, in the store
    /// or the archive; reads the demographic segments and the store only when
    /// a composition is placed.
    ///
    /// # Errors
    /// `422` naming the element and the archived EHR, the
    /// [`ServiceError::BadRequest`] of a segment that is absent or does not
    /// parse, and a storage failure.
    pub(super) async fn check_archived_episodes(
        &self,
        archive: &mut ArchiveReader,
        dir: &Path,
        manifest: &Manifest,
        format: ExportFormat,
    ) -> Result<(), ServiceError> {
        let placed = placed_ehrs(archive, dir, &manifest.segments, format)?;
        if placed.is_empty() {
            return Ok(());
        }
        let archived = archived_episodes(archive, dir, &manifest.demographic_segments, format)?;
        for (ehr_id, subject, keys) in placed {
            let person = match subject {
                Some(subject) => self.grants.person_of(&subject).await?,
                None => None,
            };
            let mut declared = BTreeSet::new();
            if let Some(person) = person {
                let rows = {
                    let mut conn = acquire(&self.demographic_pool).await?;
                    store::care_of_subject(&mut conn, person).await?
                };
                declared.extend(
                    care_of(&rows)
                        .iter()
                        .filter_map(|care| care.episode().map(|episode| episode.key)),
                );
                declared.extend(archived.get(&person).into_iter().flatten().copied());
            }
            if !keys.is_subset(&declared) {
                return Err(for_archived_ehr(episode::undeclared(), ehr_id));
            }
        }
        Ok(())
    }
}

/// The head version of each versioned object in `versions`.
fn heads(versions: &[VersionRecord]) -> impl Iterator<Item = &VersionRecord> {
    let mut by_object: BTreeMap<_, &VersionRecord> = BTreeMap::new();
    for version in versions {
        let newer = by_object
            .get(&version.vo_id)
            .is_none_or(|head| version.sys_version > head.sys_version);
        if newer {
            by_object.insert(version.vo_id, version);
        }
    }
    by_object.into_values()
}

/// The archived EHRs whose COMPOSITION versions are placed, each with its
/// promoted subject and the episodes named. A canonical-XML payload that
/// cannot be read is left to the per-record report the load gives it.
///
/// # Errors
/// `422` for a malformed placement, and the [`ServiceError::BadRequest`] of
/// a segment that is absent or does not parse.
fn placed_ehrs(
    archive: &mut ArchiveReader,
    dir: &Path,
    segments: &[String],
    format: ExportFormat,
) -> Result<Vec<Placed>, ServiceError> {
    let mut placed = Vec::new();
    for segment in segments {
        let bytes = archive.read(segment)?;
        let records: Vec<EhrRecord> =
            serde_json::from_slice(&bytes).map_err(|e| archive_refusal(dir, segment, e))?;
        for mut record in records {
            record
                .versions
                .retain(|v| v.kind == Kind::Composition.as_str());
            if format == ExportFormat::OpenehrCanonicalXml
                && resolve_version_documents(archive, &mut record).is_err()
            {
                continue;
            }
            let ehr_id = record.ehr.id;
            let mut keys = BTreeSet::new();
            for version in record.versions.iter().filter(|v| !v.body.is_null()) {
                keys.extend(
                    episode::placed(&version.body).map_err(|e| for_archived_ehr(e, ehr_id))?,
                );
            }
            if !keys.is_empty() {
                let subject = EhrSubject::stored(
                    record.ehr.subject_namespace.clone(),
                    record.ehr.subject_id.clone(),
                );
                placed.push((ehr_id, subject, keys));
            }
        }
    }
    Ok(placed)
}

/// The episodes of the care relationships the archive's demographic
/// segments carry, by the `PERSON` under care, read off each relationship's
/// head version.
///
/// # Errors
/// The [`ServiceError::BadRequest`] of a segment that is absent or does not
/// parse.
fn archived_episodes(
    archive: &mut ArchiveReader,
    dir: &Path,
    segments: &[String],
    format: ExportFormat,
) -> Result<BTreeMap<Uuid, BTreeSet<EpisodeKey>>, ServiceError> {
    let mut episodes: BTreeMap<Uuid, BTreeSet<EpisodeKey>> = BTreeMap::new();
    for segment in segments {
        let bytes = archive.read(segment)?;
        let records: Vec<DemographicRecord> =
            serde_json::from_slice(&bytes).map_err(|e| archive_refusal(dir, segment, e))?;
        for mut record in records {
            if record.kind != Kind::PartyRelationship.as_str() {
                continue;
            }
            if format == ExportFormat::OpenehrCanonicalXml
                && resolve_demographic_documents(archive, &mut record).is_err()
            {
                continue;
            }
            for head in heads(&record.versions).filter(|v| !v.body.is_null()) {
                let facts = GrantFacts::of(Kind::PartyRelationship, &head.body);
                let Some(person) = facts.care_source else {
                    continue;
                };
                episodes
                    .entry(person)
                    .or_default()
                    .extend(facts.facts.iter().filter_map(|fact| match &fact.value {
                        FactValue::Care(care) => care.episode.as_ref().map(|episode| episode.key),
                        FactValue::Code(_) | FactValue::Team(_) | FactValue::Kin(_) => None,
                    }));
            }
        }
    }
    Ok(episodes)
}
