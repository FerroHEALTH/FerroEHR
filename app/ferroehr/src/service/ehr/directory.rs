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

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::service::refusal::Missing;
use crate::service::response::ResourceMeta;
use crate::service::response::ServiceResponse;
use openehr_base::prelude::ObjectVersionId;
use openehr_its::rest::generated::common::UpdateVersion;
use openehr_rm::prelude::Folder;
use serde_json::Value;
use sqlx::Connection as _;

use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{create, delete, update};
use crate::versioning::object_version_id::{TreeId, VersionIdError, components};
use crate::versioning::read::{read_version, version_at};

use super::validation::check_folder_item_refs;
use super::{ensure_if_match, resolve_envelope};
use crate::service::datetime::parse_at_time;

impl FerroEhrService {
    /// Create the EHR's directory (its root FOLDER).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR does not exist;
    /// [`ServiceError::Unprocessable`] when the FOLDER tree is invalid;
    /// [`ServiceError::Conflict`] when the EHR is not modifiable or already
    /// has a directory; [`ServiceError::Database`] on a storage failure.
    pub(in crate::service) async fn commit_new_directory(
        &self,
        ehr_id: EhrId,
        version: UpdateVersion<Folder>,
    ) -> Result<ServiceResponse, ServiceError> {
        let version = super::canonicalize(version);
        // 553|incomplete| relaxes the existence/cardinality lower bounds (RM
        // common master06 §Incomplete Content).
        let super::CommitParts {
            audit,
            envelope,
            incomplete,
            canonical: folder,
        } = resolve_envelope(
            version,
            change_type::CREATION,
            "FOLDER directory creation",
            &self.effective_system_id(),
        )?;
        crate::storage::ehr_repo::require_ehr(&mut *acquire(&self.pool).await?, ehr_id).await?;
        self.validate_for_commit(Kind::Folder, &folder, incomplete)
            .await?;
        check_folder_item_refs(
            &mut *acquire(&self.pool).await?,
            ehr_id,
            &self.effective_system_id(),
            &folder,
        )
        .await?;
        // is_modifiable = False forbids content writes; the directory is EHR
        // content (RM ehr master04 §EHR Active Status).
        self.ensure_content_writable(ehr_id).await?;
        // `POST /directory` manages the single directory slot = EHR.directory
        // (= folders[1], RM ehr §EHR Class Directory_in_folders), conflicting
        // only when a LIVE hierarchy occupies it.
        // NOTE: no released sentence says whether a logically DELETED directory
        // still occupies the slot — our choice is that it does not block a
        // re-create, and 409 is likewise ours.
        if crate::storage::ehr_repo::live_directory_exists(&self.pool, ehr_id).await? {
            return Err(ServiceError::conflict(format!(
                "EHR {ehr_id} already has a directory"
            )));
        }

        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        let committed = create(
            &mut tx,
            Some(ehr_id),
            Kind::Folder,
            folder,
            None,
            &audit,
            envelope,
            &self.signing_ctx(),
            None,
        )
        .await?;
        tx.commit().await?;

        // The write response is metadata-only: `Committed` already carries the
        // version identity and commit instant (RM common master06 §Committal and
        // Audits), so a representation response re-reads at the protocol layer instead.
        Ok(self.committed_response(ehr_id, &committed))
    }

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
        // The current read folds the `ehr_folder` slot resolution into the
        // version statement; the as-of read keeps the two-step so the slot is
        // still chosen by CURRENT state.
        let read = match at {
            Some(at) => {
                let vo_id = self.directory_vo(ehr_id).await?;
                version_at(&self.pool, self.spec_profile, vo_id, at).await?
            }
            None => {
                crate::versioning::read::read_current_directory(
                    &self.pool,
                    self.spec_profile,
                    ehr_id,
                )
                .await?
            }
        }
        .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::Folder)
        .ok_or_else(|| {
            ServiceError::not_found(
                Missing::VersionedObject,
                format!("directory for EHR {ehr_id}"),
            )
        })?;
        if read.deleted() {
            return Ok(ServiceResponse::plain(Value::Null));
        }
        let vo_id = read.vo_id;
        let meta = self.version_meta(
            ehr_id,
            vo_id,
            &read.creating_system_id,
            read.tree,
            read.time_committed,
        );
        let folder = self.with_uid(read.canonical, vo_id, &read.creating_system_id, read.tree)?;
        match path.map(str::trim).filter(|p| !p.is_empty() && *p != "/") {
            None => Ok(ServiceResponse::new(folder, meta)),
            Some(path) => select_subfolder(&folder, path)
                .map(|sub| ServiceResponse::new(sub, meta))
                .ok_or_else(|| {
                    ServiceError::not_found(
                        Missing::VersionedObject,
                        format!("folder path {path:?}"),
                    )
                }),
        }
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
    ) -> Result<ServiceResponse, ServiceError> {
        let read = read_version(&self.pool, self.spec_profile, vo_id, version)
            .await?
            .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == Kind::Folder)
            .ok_or_else(|| {
                ServiceError::not_found(Missing::Version, format!("directory {vo_id} v{version}"))
            })?;
        if read.deleted() {
            return Ok(ServiceResponse::plain(Value::Null));
        }
        let mut response = self.version_response(ehr_id, vo_id, read)?;
        if let Some(path) = path.map(str::trim).filter(|p| !p.is_empty() && *p != "/") {
            response.body = select_subfolder(&response.body, path).ok_or_else(|| {
                ServiceError::not_found(Missing::VersionedObject, format!("folder path {path:?}"))
            })?;
        }
        Ok(response)
    }

    /// Update the EHR's directory. `vo_id` is the directory-slot versioned
    /// object (resolved once by the caller's `If-Match` meta pre-read, so the
    /// JOIN is not re-run here); `is_modifiable` is the EHR's content-write
    /// flag from that same merged pre-read (so the writability probe is not
    /// re-run either); `expected` (from `If-Match`) enforces optimistic
    /// concurrency.
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] when the FOLDER tree is invalid;
    /// [`ServiceError::Conflict`] when the EHR is not modifiable or the
    /// optimistic lock fails; [`ServiceError::Database`] on a storage
    /// failure.
    pub(in crate::service) async fn commit_directory_update(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        version: UpdateVersion<Folder>,
        expected: Option<TreeId>,
        is_modifiable: bool,
    ) -> Result<ServiceResponse, ServiceError> {
        let version = super::canonicalize(version);
        let super::CommitParts {
            audit,
            envelope,
            incomplete,
            canonical: folder,
        } = resolve_envelope(
            version,
            change_type::MODIFICATION,
            "FOLDER directory update",
            &self.effective_system_id(),
        )?;
        self.validate_for_commit(Kind::Folder, &folder, incomplete)
            .await?;
        check_folder_item_refs(
            &mut *acquire(&self.pool).await?,
            ehr_id,
            &self.effective_system_id(),
            &folder,
        )
        .await?;
        // is_modifiable = False forbids content writes to the directory (RM ehr
        // master04 §EHR Active Status), after validate_folder's 422.
        if !is_modifiable {
            return Err(crate::versioning::change::not_modifiable_error(ehr_id));
        }

        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        let committed = update(
            &mut tx,
            Some(ehr_id),
            vo_id,
            Kind::Folder,
            folder,
            expected,
            None,
            &audit,
            envelope,
            &self.signing_ctx(),
        )
        .await?;
        tx.commit().await?;

        Ok(self.committed_response(ehr_id, &committed))
    }

    /// Logically delete the EHR's directory. `204_because_deleted` declares no
    /// `ETag`/`Location`, so the response carries no metadata. `is_modifiable`
    /// is the EHR's content-write flag from the caller's merged pre-read (so
    /// the writability probe is not re-run here).
    ///
    /// # Errors
    /// [`ServiceError::Conflict`] when the EHR is not modifiable or the
    /// optimistic lock fails; [`ServiceError::Database`] on a storage
    /// failure.
    pub(in crate::service) async fn delete_directory_at(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        expected: Option<TreeId>,
        is_modifiable: bool,
        update_audit: Option<&openehr_its::rest::generated::common::UpdateAudit>,
    ) -> Result<ServiceResponse, ServiceError> {
        // is_modifiable = False forbids content writes (RM ehr master04 §EHR
        // Active Status).
        if !is_modifiable {
            return Err(crate::versioning::change::not_modifiable_error(ehr_id));
        }

        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        // The committal request headers merge into the delete audit too
        // (ITS-REST overview §"openehr-version and openehr-audit-details":
        // accepted on PUT, POST and DELETE).
        let audit = match update_audit {
            Some(u) => crate::versioning::audit::AuditInput::from_update(
                u,
                change_type::DELETED,
                "DIRECTORY delete",
                &self.effective_system_id(),
            )?,
            None => self.audit(change_type::DELETED, "DIRECTORY delete"),
        };
        let committed = delete(
            &mut tx,
            Some(ehr_id),
            vo_id,
            Kind::Folder,
            expected,
            &audit,
            None,
            &self.signing_ctx(),
        )
        .await?;
        tx.commit().await?;
        // The 204's identity: the NEW 523|deleted| version the delete
        // committed (RM common master06 §Logical Deletion) — the resource's
        // current state, carried so the wire serves the weak ETag +
        // Last-Modified the overview §"ETag and Last-Modified" SHOULDs on
        // versioned resources.
        Ok(self.committed_response(ehr_id, &committed))
    }

    /// The current directory FOLDER version metadata of an EHR, or `None` when
    /// the EHR indexes no directory hierarchy.
    ///
    /// # Errors
    /// [`ServiceError`] if the metadata read fails.
    pub async fn directory_latest_meta(
        &self,
        ehr_id: EhrId,
    ) -> Result<Option<ResourceMeta>, ServiceError> {
        Ok(self
            .directory_meta_with_vo(ehr_id)
            .await?
            .map(|(_, _, m)| m))
    }

    /// The directory-slot versioned-object id, the EHR's `is_modifiable`
    /// content-write flag, **and** the current version metadata, resolved and
    /// read in ONE `ehr_folder`⋈`version`⋈`audit`⋈`ehr` statement
    /// ([`crate::storage::ehr_repo::directory_current_meta`], with no node
    /// reassembly and no attestation read). Threading the `vo_id` and
    /// `is_modifiable` back to the caller lets the inner write skip re-running
    /// the slot JOIN and the writability probe. `None` when the EHR indexes no
    /// directory hierarchy.
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the merged read fails.
    pub(in crate::service) async fn directory_meta_with_vo(
        &self,
        ehr_id: EhrId,
    ) -> Result<Option<(VoId, bool, ResourceMeta)>, ServiceError> {
        let Some((m, is_modifiable)) =
            crate::storage::ehr_repo::directory_current_meta(&self.pool, ehr_id).await?
        else {
            return Ok(None);
        };
        let tree = TreeId::from_columns(m.trunk_version, m.branch_number, m.branch_version);
        let meta = self.version_meta(
            ehr_id,
            m.vo_id,
            &m.creating_system_id,
            tree,
            m.time_committed,
        );
        Ok(Some((m.vo_id, is_modifiable, meta)))
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
        a_dir_struct: UpdateVersion<Folder>,
    ) -> Result<ResourceMeta, ServiceError> {
        super::committed_meta(
            self.commit_new_directory(an_ehr_id, a_dir_struct)
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
        a_dir_struct: UpdateVersion<Folder>,
    ) -> Result<ResourceMeta, ServiceError> {
        // The slot vo_id, its current version metadata and the EHR's
        // is_modifiable flag resolve ONCE here and are threaded into the write,
        // so neither the slot JOIN nor the writability probe runs twice.
        let Some((vo_id, is_modifiable, latest)) = self.directory_meta_with_vo(an_ehr_id).await?
        else {
            return Err(ServiceError::not_found(
                Missing::VersionedObject,
                format!("directory for EHR {an_ehr_id}"),
            ));
        };
        ensure_if_match(a_dir_struct.preceding_version_uid.as_ref(), Some(&latest))?;
        let expected = a_dir_struct
            .preceding_version_uid
            .as_ref()
            .map(|o| components(o).map(|(_, v)| v))
            .transpose()
            .map_err(VersionIdError::into_argument)?;
        super::committed_meta(
            self.commit_directory_update(an_ehr_id, vo_id, a_dir_struct, expected, is_modifiable)
                .await
                .map_err(ServiceError::flattened)?,
        )
    }

    /// Logically delete the EHR's
    /// directory (a new `523|deleted|` version, RM common master06 §Logical
    /// Deletion). Returns the committed deleted version's metadata (uid +
    /// commit instant) so the wire 204 carries the weak `ETag`/`Last-Modified`
    /// the overview §"`ETag` and Last-Modified" SHOULDs on versioned
    /// resources.
    ///
    /// # Errors
    /// [`ServiceError`] when the EHR has no directory (404-equivalent), the
    /// `preceding_version_uid` mismatches the current latest (412-equivalent),
    /// the EHR is not modifiable (409-equivalent), or the commit fails.
    pub async fn delete_directory(
        &self,
        an_ehr_id: EhrId,
        preceding_version_uid: Option<ObjectVersionId>,
        update_audit: Option<&openehr_its::rest::generated::common::UpdateAudit>,
    ) -> Result<ServiceResponse, ServiceError> {
        let Some((vo_id, is_modifiable, latest)) = self.directory_meta_with_vo(an_ehr_id).await?
        else {
            return Err(ServiceError::not_found(
                Missing::VersionedObject,
                format!("directory for EHR {an_ehr_id}"),
            ));
        };
        ensure_if_match(preceding_version_uid.as_ref(), Some(&latest))?;
        let expected = preceding_version_uid
            .as_ref()
            .map(|o| components(o).map(|(_, v)| v))
            .transpose()
            .map_err(VersionIdError::into_argument)?;
        let resp = self
            .delete_directory_at(an_ehr_id, vo_id, expected, is_modifiable, update_audit)
            .await?;
        Ok(resp)
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
    ) -> Result<ServiceResponse, ServiceError> {
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
