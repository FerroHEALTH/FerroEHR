// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The DIRECTORY (FOLDER) surface, one hierarchy per EHR through the ITS-REST
//! `/directory` binding.
//!
//! Spec: RM ehr `master04-ehr_package.adoc` §Folders + the EHR class
//! invariants `Directory_in_folders` (`EHR.directory = folders.item(1)`) /
//! `Folders_valid`; RM common `org.openehr.rm.common.folder.adoc`.
//! Versioned-object mechanics are RM common master06, delegated to
//! [`crate::versioning`]; the FOLDER-tree commit validation lives in
//! [`validation`](super::validation).
//!
//! NOTE: the read side is multi-hierarchy (the `ehr_read` folder refs) while
//! the write side manages the single directory slot (`folders[1]`) only —
//! additional `EHR.folders` hierarchies are committable through a CONTRIBUTION,
//! the only committal path the release describes for them (RM ehr
//! `master04-ehr_package.adoc` §Folders declares `EHR.folders`
//! `List<VERSIONED_FOLDER>` with `EHR.directory = folders.item(1)`). No openEHR
//! spec governs a multi-hierarchy directory WRITE surface.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};
use crate::service::response::ResourceMeta;
use crate::service::response::{RawServiceResponse, ReadBody, ServiceResponse};
use openehr_base::prelude::ObjectVersionId;
use openehr_rm::prelude::Folder;
use serde_json::Value;

use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::service::version_update::WriteRequest;
use crate::storage::version_repo::read::VersionAt;
use crate::versioning::Kind;
use crate::versioning::object_version_id::{TreeId, VersionIdError, components};

use crate::service::datetime::parse_at_time;

impl FerroEhrService {
    /// The EHR's directory FOLDER (current, or at an instant when `at` is
    /// given), optionally navigated to a sub-folder `path` (`/a/b`). A deleted
    /// directory resolves to `Value::Null` (→ 204).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR has no directory, no version
    /// existed at `at`, or the sub-folder path does not resolve;
    /// [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn directory_at_time(
        &self,
        ehr_id: EhrId,
        at: Option<jiff::Timestamp>,
        path: Option<&str>,
    ) -> Result<ServiceResponse, ServiceError> {
        self.directory_at_time_raw(ehr_id, at, path)
            .await?
            .into_value_response()
            .map_err(|e| ServiceError::internal("read a stored directory body", e))
    }

    /// [`Self::directory_at_time`], with the body kept as the stored text
    /// where nothing needs it parsed ([`crate::versioning::read::read_raw`]).
    ///
    /// # Errors
    /// As [`Self::directory_at_time`].
    pub(in crate::service) async fn directory_at_time_raw(
        &self,
        ehr_id: EhrId,
        at: Option<jiff::Timestamp>,
        path: Option<&str>,
    ) -> Result<RawServiceResponse, ServiceError> {
        // The current read folds the `ehr_folder` slot resolution into the
        // version statement; the as-of read keeps the two-step so the slot is
        // still chosen by CURRENT state.
        let at = match at {
            Some(at) => VersionAt::Instant(self.directory_vo(ehr_id).await?, at),
            None => VersionAt::Directory(ehr_id),
        };
        let raw = crate::versioning::read::read_raw(&self.pool, self.spec_profile, at)
            .await?
            .filter(|r| r.read.ehr_id == Some(ehr_id) && r.read.kind == Kind::Folder)
            .ok_or_else(|| {
                ServiceError::not_found(
                    Missing::VersionedObject,
                    format!("directory for EHR {ehr_id}"),
                )
            })?;
        if raw.read.deleted() {
            return Ok(deleted_folder());
        }
        let read = &raw.read;
        let meta = self.version_meta(
            ehr_id,
            read.vo_id,
            &read.creating_system_id,
            read.tree,
            read.time_committed,
        );
        self.folder_served(raw, meta, path)
    }

    /// A specific version of the directory (from a `version_uid`),
    /// optionally navigated to a sub-folder `path` (ITS-REST
    /// `directory_get_by_version_id` — "If `path` is supplied, retrieves
    /// from the directory only the sub-FOLDER that is associated with that
    /// path"). A deleted version resolves to `Value::Null` (→ 204).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the version does not exist, belongs
    /// to another EHR, or the sub-folder path does not resolve;
    /// [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn directory_version(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        version: TreeId,
        path: Option<&str>,
    ) -> Result<RawServiceResponse, ServiceError> {
        let raw = self
            .tree_version_raw(ehr_id, Kind::Folder, vo_id, version)
            .await?
            .ok_or_else(|| {
                ServiceError::not_found(Missing::Version, format!("directory {vo_id} v{version}"))
            })?;
        if raw.read.deleted() {
            return Ok(deleted_folder());
        }
        let read = &raw.read;
        let meta = self
            .version_meta(
                ehr_id,
                vo_id,
                &read.creating_system_id,
                read.tree,
                read.time_committed,
            )
            .with_content(vec![read.content.clone()]);
        self.folder_served(raw, meta, path)
    }

    /// The directory read of `raw` with the metadata `meta`, navigated to the
    /// sub-folder `path` when one is named, which parses the stored text.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the sub-folder path does not resolve,
    /// and the identifier and body faults of the stored version.
    fn folder_served(
        &self,
        raw: crate::versioning::read::RawVersionRead,
        meta: ResourceMeta,
        path: Option<&str>,
    ) -> Result<RawServiceResponse, ServiceError> {
        let vo_id = raw.read.vo_id;
        let body = self.served_body(vo_id, raw.read, raw.raw_json)?;
        let Some(path) = path.map(str::trim).filter(|p| !p.is_empty() && *p != "/") else {
            return Ok(RawServiceResponse {
                body,
                meta: Some(meta),
            });
        };
        let folder = body
            .into_value()
            .map_err(|e| ServiceError::internal("read a stored directory body", e))?;
        let sub = select_subfolder(&folder, path).ok_or_else(|| {
            ServiceError::not_found(Missing::VersionedObject, format!("folder path {path:?}"))
        })?;
        Ok(RawServiceResponse {
            body: ReadBody::Value(sub),
            meta: Some(meta),
        })
    }

    /// The current directory FOLDER version metadata of an EHR, or `None` when
    /// the EHR indexes no directory hierarchy: the slot and its current
    /// version read in one statement
    /// ([`crate::storage::ehr_repo::directory_current_meta`], with no node
    /// reassembly and no attestation read).
    ///
    /// # Errors
    /// [`ServiceError`] if the metadata read fails.
    pub async fn directory_latest_meta(
        &self,
        ehr_id: EhrId,
    ) -> Result<Option<ResourceMeta>, ServiceError> {
        let Some(m) = crate::storage::ehr_repo::directory_current_meta(&self.pool, ehr_id).await?
        else {
            return Ok(None);
        };
        let tree = TreeId::from_columns(m.trunk_version, m.branch_number, m.branch_version);
        Ok(Some(self.version_meta(
            ehr_id,
            m.vo_id,
            &m.creating_system_id,
            tree,
            m.time_committed,
        )))
    }

    /// The versioned-object id of the EHR's directory — `EHR.directory`
    /// (= `folders.item(1)`, RM ehr §EHR Class `Directory_in_folders`).
    /// Resolved as the lowest-`rank` LIVE hierarchy; when none is live it
    /// falls back to the lowest-`rank` hierarchy that still exists, so a read
    /// after a logical delete resolves to the deleted version (→ 204) rather
    /// than 404. `None` when the EHR indexes no folder hierarchy.
    ///
    /// The `ehr_folder` ⋈ `version` resolution is a storage seam
    /// ([`crate::storage::ehr_repo::directory_vo`]; no openEHR spec governs
    /// the SQL — our own design).
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the slot resolution fails.
    pub(in crate::service) async fn directory_vo_opt(
        &self,
        ehr_id: EhrId,
    ) -> Result<Option<VoId>, ServiceError> {
        Ok(crate::storage::ehr_repo::directory_vo(&self.pool, ehr_id).await?)
    }

    /// The EHR's directory versioned-object id, or `NotFound`.
    async fn directory_vo(&self, ehr_id: EhrId) -> Result<VoId, ServiceError> {
        self.directory_vo_opt(ehr_id).await?.ok_or_else(|| {
            ServiceError::not_found(
                Missing::VersionedObject,
                format!("directory for EHR {ehr_id}"),
            )
        })
    }
}

/// Navigate a FOLDER tree to a sub-folder by name path (`a/b/c`), matching
/// each segment against child `folders[].name.value` (ITS-REST
/// `parameters/query/path.yaml`: slash-separated folder names). Returns the
/// sub-folder JSON.
fn select_subfolder(folder: &Value, path: &str) -> Option<Value> {
    let mut current = folder;
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        let children = current.get("folders")?.as_array()?;
        current = children.iter().find(|f| {
            f.get("name")
                .and_then(|n| n.get("value"))
                .and_then(Value::as_str)
                == Some(segment)
        })?;
    }
    Some(current.clone())
}

// ── The DIRECTORY call surface ────────────────────────────────────────────────

impl FerroEhrService {
    /// Create the EHR's directory,
    /// returning the new version's resource metadata (uid + commit time for
    /// the wire's `ETag`/`Last-Modified`).
    ///
    /// # Errors
    /// [`ServiceError`] when the EHR does not exist (404-equivalent), the FOLDER
    /// tree is invalid (422-equivalent), the EHR is not modifiable or already
    /// has a directory (409-equivalent), or the commit fails.
    pub async fn create_directory(
        &self,
        an_ehr_id: EhrId,
        request: WriteRequest<Folder>,
    ) -> Result<ResourceMeta, ServiceError> {
        super::committed_meta(
            self.commit_new_directory(an_ehr_id, request)
                .await
                .map_err(ServiceError::flattened)?,
        )
    }

    /// The directory FOLDER
    /// current at `a_time` (or now), optionally navigated to `a_path`. The
    /// response carries the resource metadata so the wire can emit
    /// `ETag`/`Last-Modified` on reads too (ITS-REST overview §"`ETag` and
    /// Last-Modified": both SHOULD accompany versioned resources).
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed `a_time` (400-equivalent), a missing
    /// directory/version/path (404-equivalent), or a read failure.
    pub async fn get_directory_at_time(
        &self,
        an_ehr_id: EhrId,
        a_time: Option<String>,
        a_path: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        let at = a_time.as_deref().map(parse_at_time).transpose()?;
        self.directory_at_time(an_ehr_id, at, a_path.as_deref())
            .await
    }

    /// [`Self::get_directory_at_time`], with the body kept as the stored text
    /// where nothing needs it parsed.
    ///
    /// # Errors
    /// As [`Self::get_directory_at_time`].
    pub async fn get_directory_at_time_raw(
        &self,
        an_ehr_id: EhrId,
        a_time: Option<String>,
        a_path: Option<String>,
    ) -> Result<RawServiceResponse, ServiceError> {
        let at = a_time.as_deref().map(parse_at_time).transpose()?;
        self.directory_at_time_raw(an_ehr_id, at, a_path.as_deref())
            .await
    }

    /// Commit a new directory version,
    /// returning the new version's resource metadata (uid + commit time for
    /// the wire's `ETag`/`Last-Modified`).
    ///
    /// # Errors
    /// [`ServiceError`] when the EHR has no directory (404-equivalent), the
    /// `preceding_version_uid` mismatches the current latest (412-equivalent),
    /// the FOLDER tree is invalid (422-equivalent), the EHR is not modifiable
    /// (409-equivalent), or the commit fails.
    pub async fn update_directory(
        &self,
        an_ehr_id: EhrId,
        request: WriteRequest<Folder>,
    ) -> Result<ResourceMeta, ServiceError> {
        super::committed_meta(
            self.commit_directory_update(an_ehr_id, request)
                .await
                .map_err(ServiceError::flattened)?,
        )
    }

    /// The directory FOLDER at the named version
    /// (`GET /ehr/{ehr_id}/directory/{version_uid}`), optionally navigated
    /// to the sub-folder `a_path` (ITS-REST `directory_get_by_version_id`:
    /// slash-separated FOLDER names; an unresolved path is 404-equivalent).
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed `OBJECT_VERSION_ID`, an unknown version
    /// or unresolved path (404-equivalent), or a read failure.
    pub async fn get_directory_at_version(
        &self,
        an_ehr_id: EhrId,
        a_version_uid: ObjectVersionId,
        a_path: Option<&str>,
    ) -> Result<RawServiceResponse, ServiceError> {
        let (vo_id, version) = components(&a_version_uid).map_err(VersionIdError::into_argument)?;
        let resp = self
            .directory_version(an_ehr_id, vo_id, version, a_path)
            .await?;
        // The addressed version_uid must equal the stored full three-part
        // identity (ITS-REST overview Resources.md §Identifier types; BASE
        // master05 case rule) — a fabricated creating_system_id names no
        // VERSION here. A deleted read carries no metadata (204 body-less),
        // so there is nothing to verify against.
        if let Some(meta) = resp.meta.as_ref() {
            super::ensure_addressed_version(&a_version_uid, &meta.uid)?;
        }
        Ok(resp)
    }
}

/// The read of a logically deleted directory: no body (→ `204`).
fn deleted_folder() -> RawServiceResponse {
    RawServiceResponse {
        body: ReadBody::Value(Value::Null),
        meta: None,
    }
}
