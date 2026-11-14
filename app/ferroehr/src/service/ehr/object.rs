// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads of `EHR_STATUS` and `EHR_ACCESS`, the objects an EHR holds one of.
//!
//! Each serves the object current at an instant or at a version, its
//! versioned object, its revision history and its VERSION envelopes.
//!
//! Spec: RM ehr `master04-ehr_package.adoc` §EHR Status and §EHR Access (the
//! EHR class holds `ehr_status` 1..1 and `ehr_access` 1..1, each versioned
//! "via the normal mechanism"); the versioned-object mechanics are RM common
//! master06, delegated to [`crate::versioning`]. ITS-REST 1.1.0 serves these
//! reads for `EHR_STATUS` only; FerroEHR serves the same reads for
//! `EHR_ACCESS` as its own extension (#5517), to the callers who may read
//! its settings: an administrator, the subject and a gate-keeper
//! ([`FerroEhrService::may_read_ehr_access`]).

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::datetime::parse_at_time;
use crate::service::response::ServiceResponse;
use crate::storage::version_repo::read::VersionAt;
use crate::versioning::Kind;
use crate::versioning::object_version_id::{VersionIdError, parse_tree_id};
use crate::versioning::read::VersionRead;
use crate::versioning::wire::{revision_history, versioned_object};

/// An object an EHR holds exactly one of (RM ehr, EHR class).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EhrObject {
    /// The EHR's `EHR_STATUS`.
    Status,
    /// The EHR's `EHR_ACCESS`.
    Access,
}

impl EhrObject {
    /// The versioned-object kind the object is stored under.
    #[must_use]
    pub const fn kind(self) -> Kind {
        match self {
            EhrObject::Status => Kind::EhrStatus,
            EhrObject::Access => Kind::EhrAccess,
        }
    }

    /// The RM class name of the object.
    #[must_use]
    pub const fn rm_type(self) -> &'static str {
        match self {
            EhrObject::Status => "EHR_STATUS",
            EhrObject::Access => "EHR_ACCESS",
        }
    }

    /// The RM class name of its versioned object.
    #[must_use]
    pub const fn versioned_rm_type(self) -> &'static str {
        match self {
            EhrObject::Status => "VERSIONED_EHR_STATUS",
            EhrObject::Access => "VERSIONED_EHR_ACCESS",
        }
    }
}

impl FerroEhrService {
    /// The `object` of an EHR with its `uid` set, the current version or the
    /// one current at `a_time` (time-travel), with the version metadata the
    /// wire's `ETag`/`Last-Modified` need.
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed `a_time` (400-equivalent);
    /// [`ServiceError::NotFound`] when the EHR has no current `object` or
    /// none existed at that instant; [`ServiceError::Database`] on a storage
    /// failure.
    pub async fn ehr_object(
        &self,
        ehr_id: EhrId,
        object: EhrObject,
        a_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        self.ensure_object_readable(ehr_id, object).await?;
        let kind = object.kind().as_str();
        let at = match a_time.as_deref().map(parse_at_time).transpose()? {
            Some(at) => VersionAt::InstantOfKind(ehr_id, kind, at),
            None => VersionAt::CurrentOfKind(ehr_id, kind),
        };
        // ONE statement: the container resolution and the version read merged
        // (an EHR holds exactly one container of each kind, RM ehr `ehr.adoc`).
        let read = crate::versioning::read::read(&self.pool, self.spec_profile, at)
            .await?
            .ok_or_else(|| missing_object(ehr_id, object))?;
        let vo_id = read.vo_id;
        Ok(self.version_response(ehr_id, vo_id, read)?)
    }

    /// The **bare** `object` at a specific version (not the
    /// `ORIGINAL_VERSION` wrapper), with the version metadata the wire's
    /// `ETag`/`Last-Modified` need. `a_version` is the `VERSION_TREE_ID`
    /// lexical form.
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed version id;
    /// [`ServiceError::NotFound`] when the version does not exist, belongs to
    /// another EHR or is of another kind; [`ServiceError::Database`] on a
    /// storage failure.
    pub async fn ehr_object_at_version(
        &self,
        ehr_id: EhrId,
        object: EhrObject,
        vo_id: VoId,
        a_version: &str,
    ) -> Result<ServiceResponse, ServiceError> {
        self.ensure_object_readable(ehr_id, object).await?;
        let read = self
            .object_version_read(ehr_id, object, vo_id, a_version)
            .await?;
        Ok(self.version_response(ehr_id, vo_id, read)?)
    }

    /// The versioned object of the EHR's `object` with the container
    /// metadata the wire's `ETag`/`Last-Modified` need: the container uid
    /// identity plus the newest held version's commit instant (ITS-REST
    /// overview `Requests_and_responses.md` §"`ETag` and Last-Modified": both
    /// headers SHOULD accompany a `VERSIONED_OBJECT` response).
    ///
    /// # Errors
    /// [`ServiceError`] when the EHR has no current `object` (404-equivalent)
    /// or a read fails.
    pub async fn versioned_ehr_object(
        &self,
        ehr_id: EhrId,
        object: EhrObject,
    ) -> Result<ServiceResponse, ServiceError> {
        self.ensure_object_readable(ehr_id, object).await?;
        let vo_id = self.object_vo(ehr_id, object).await?;
        let (body, last_modified) =
            versioned_object(&self.pool, vo_id, ehr_id, object.versioned_rm_type())
                .await?
                .ok_or_else(|| {
                    ServiceError::not_found(
                        Missing::VersionedObject,
                        format!("versioned object {vo_id}"),
                    )
                })?;
        Ok(ServiceResponse::new(
            body,
            super::meta::container_meta(ehr_id, vo_id, last_modified),
        ))
    }

    /// The `REVISION_HISTORY` of the EHR's `object` (RM common
    /// `versioned_object.adoc` `revision_history()`) with the container
    /// metadata of [`Self::versioned_ehr_object`].
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR has no current `object`;
    /// [`ServiceError::Database`] on a storage failure.
    pub async fn ehr_object_revision_history(
        &self,
        ehr_id: EhrId,
        object: EhrObject,
    ) -> Result<ServiceResponse, ServiceError> {
        self.ensure_object_readable(ehr_id, object).await?;
        let vo_id = self.object_vo(ehr_id, object).await?;
        let (body, last_modified) = revision_history(&self.pool, ehr_id, vo_id).await?;
        Ok(ServiceResponse::new(
            body,
            super::meta::container_meta(ehr_id, vo_id, last_modified),
        ))
    }

    /// The VERSION envelope of the EHR's `object` at a specific version: an
    /// `ORIGINAL_VERSION`, or an `IMPORTED_VERSION` when the version was
    /// received from another system (RM common master06 §Version and its
    /// Subtypes), with the version metadata the wire's `ETag`/`Last-Modified`
    /// need. `a_version` is the `VERSION_TREE_ID` lexical form.
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed version id;
    /// [`ServiceError::NotFound`] when the version does not exist, belongs to
    /// another EHR or is of another kind; [`ServiceError::Database`] on a
    /// storage failure.
    pub async fn ehr_object_version_envelope(
        &self,
        ehr_id: EhrId,
        object: EhrObject,
        vo_id: VoId,
        a_version: &str,
    ) -> Result<ServiceResponse, ServiceError> {
        self.ensure_object_readable(ehr_id, object).await?;
        let read = self
            .object_version_read(ehr_id, object, vo_id, a_version)
            .await?;
        self.envelope_response(ehr_id, &read)
    }

    /// The VERSION envelope of the EHR's `object` extant at `a_time`, or the
    /// latest when `a_time` is `None`, with the version metadata the wire's
    /// `ETag`/`Last-Modified` need.
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed `a_time` (400-equivalent);
    /// [`ServiceError::NotFound`] when the EHR has no current `object` or
    /// none existed at that instant; [`ServiceError::Database`] on a storage
    /// failure.
    pub async fn ehr_object_version_at_time(
        &self,
        ehr_id: EhrId,
        object: EhrObject,
        a_time: Option<String>,
    ) -> Result<ServiceResponse, ServiceError> {
        self.ensure_object_readable(ehr_id, object).await?;
        let at = a_time.as_deref().map(parse_at_time).transpose()?;
        let vo_id = self.object_vo(ehr_id, object).await?;
        let at = match at {
            Some(at) => VersionAt::Instant(vo_id, at),
            None => VersionAt::Current(vo_id),
        };
        let read = crate::versioning::read::read(&self.pool, self.spec_profile, at)
            .await?
            .ok_or_else(|| {
                ServiceError::not_found(
                    Missing::ObjectVersion,
                    format!("{} version at time for EHR {ehr_id}", object.rm_type()),
                )
            })?;
        self.envelope_response(ehr_id, &read)
    }

    /// Refuses a read of the `EHR_ACCESS` of `ehr_id` by a caller who may not
    /// read its settings ([`Self::may_read_ehr_access`]) as a read of an EHR
    /// that holds none, as the CONTRIBUTION and extract reads hide it, since
    /// a restriction must not be visible to a provider (EHDS Art. 8); our own
    /// design.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] for such a caller; the refusals of
    /// [`Self::may_read_ehr_access`].
    async fn ensure_object_readable(
        &self,
        ehr_id: EhrId,
        object: EhrObject,
    ) -> Result<(), ServiceError> {
        if object == EhrObject::Access && !Box::pin(self.may_read_ehr_access(ehr_id)).await? {
            return Err(missing_object(ehr_id, object));
        }
        Ok(())
    }

    /// The `vo_id` of the EHR's `object` container.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the EHR has no current `object`;
    /// [`ServiceError::Database`] on a storage failure.
    async fn object_vo(&self, ehr_id: EhrId, object: EhrObject) -> Result<VoId, ServiceError> {
        let (vo_id, _) = self
            .current_vo(ehr_id, object.kind())
            .await?
            .ok_or_else(|| missing_object(ehr_id, object))?;
        Ok(vo_id)
    }

    /// One version of the EHR's `object`, addressed by its container and
    /// `VERSION_TREE_ID` lexical form.
    ///
    /// # Errors
    /// [`ServiceError`] for a malformed version id;
    /// [`ServiceError::NotFound`] when the version does not exist, belongs to
    /// another EHR or is of another kind; [`ServiceError::Database`] on a
    /// storage failure.
    async fn object_version_read(
        &self,
        ehr_id: EhrId,
        object: EhrObject,
        vo_id: VoId,
        a_version: &str,
    ) -> Result<VersionRead, ServiceError> {
        let version = parse_tree_id(a_version).map_err(VersionIdError::into_argument)?;
        crate::versioning::read::read(
            &self.pool,
            self.spec_profile,
            VersionAt::Tree(vo_id, version),
        )
        .await?
        .filter(|r| r.ehr_id == Some(ehr_id) && r.kind == object.kind())
        .ok_or_else(|| {
            ServiceError::not_found(
                Missing::ObjectVersion,
                format!("{} {vo_id} v{version}", object.rm_type()),
            )
        })
    }
}

/// The `404` of an EHR that holds no current `object`.
fn missing_object(ehr_id: EhrId, object: EhrObject) -> ServiceError {
    ServiceError::not_found(
        Missing::Ehr,
        format!("{} for EHR {ehr_id}", object.rm_type()),
    )
}
