// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Local resolution of `ehr:` URIs (`DV_EHR_URI` / `LOCATABLE_REF`) to stored
//! canonical-JSON content, over the versioned-object read surface.
//!
//! BASE `docs/architecture_overview/master11-paths.adoc` §"EHR URIs" defines the
//! URI grammar, parsed by [`openehr_rm::v1_2::paths::EhrUri`], and leaves
//! resolution to an unspecified name-resolution service ("Until such services
//! are established, ad hoc means of dealing with `ehr:` URIs are likely to be
//! used"). No openEHR spec governs how a server resolves such a URI to a node —
//! the local resolution here is our own extension over the same
//! versioned-object machinery the REST reads use. Foreign-system resolution is
//! out of scope.
//!
//! The item-path portion is applied with the RM `PATHABLE` primitives
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.pathable.adoc`):
//! [`FerroEhrService::resolve_ehr_uri`] enforces the `item_at_path`
//! precondition `path_unique`, while
//! [`FerroEhrService::resolve_ehr_uri_items`] returns every match
//! (`items_at_path`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use openehr_rm::v1_2::paths::{EhrUri, TopLevelLocator, VersionLocator};
use serde_json::Value;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::service::status::CallStatusType;
use crate::versioning::Kind;
use crate::versioning::object_version_id::{TreeId, components};
use crate::versioning::read::{read_current, read_version};

impl FerroEhrService {
    /// Resolve an `ehr:` URI to the single canonical-JSON node it addresses.
    ///
    /// The item-path portion, if present, must resolve to exactly one node
    /// (the RM `PATHABLE.item_at_path` precondition `path_unique`); a
    /// non-unique path is a [`ServiceError::BadRequest`] and an empty result a
    /// [`ServiceError::NotFound`]. With no item path the whole top-level
    /// object is returned.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when the URI names a foreign system, an
    /// unknown EHR/object/version, or resolves to no item;
    /// [`ServiceError::BadRequest`] for a relative URI (no EHR context) or a
    /// non-unique item path.
    pub async fn resolve_ehr_uri(&self, uri: &EhrUri) -> Result<Value, ServiceError> {
        let mut items = self.resolve_ehr_uri_items(uri).await?;
        match items.len() {
            0 => Err(ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("ehr: URI {uri} resolved to no item"),
            )),
            1 => Ok(items.remove(0)),
            n => Err(ServiceError::precondition(format!(
                "ehr: URI {uri} path is not unique ({n} matches); item_at_path requires \
                 path_unique (RM common pathable)"
            ))),
        }
    }

    /// Resolve an `ehr:` URI to *every* canonical-JSON node its path addresses
    /// (RM `PATHABLE.items_at_path`). With no item path the result is the
    /// single top-level object; with no locator it is the EHR object.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] for a foreign system id or an unknown
    /// EHR/object/version; [`ServiceError::BadRequest`] for a relative URI
    /// with no EHR context.
    pub async fn resolve_ehr_uri_items(&self, uri: &EhrUri) -> Result<Vec<Value>, ServiceError> {
        // Foreign-system resolution is out of scope (master11 §"EHR URIs": name
        // resolution across systems is unspecified). Our own extension resolves
        // the local system only.
        if let Some(system) = &uri.system_id
            && !system.eq_ignore_ascii_case(&self.effective_system_id())
        {
            return Err(ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!(
                    "ehr: URI names foreign system {system:?}; only the local system \
                     ({}) is resolvable",
                    self.effective_system_id()
                ),
            ));
        }
        // A relative URI (no ehr_id) carries no EHR context to resolve against.
        let ehr_id = EhrId(uri.ehr_id.ok_or_else(|| {
            ServiceError::precondition(
                "relative ehr: URI (no ehr_id) has no EHR context to resolve against".to_owned(),
            )
        })?);
        self.ensure_ehr_exists(ehr_id).await?;

        // A bare `ehr:/ehr_id` "refers to an EHR" (master11 §"EHR Location") —
        // surface the EHR object.
        let Some(locator) = &uri.locator else {
            return Ok(vec![self.ehr_summary(ehr_id).await?.body]);
        };

        let object = self.resolve_locator(ehr_id, locator).await?;

        match &uri.item_path {
            None => Ok(vec![object]),
            Some(path) => Ok(openehr_rm::v1_2::paths::items_at_path(&object, path)
                .into_iter()
                .cloned()
                .collect()),
        }
    }

    /// Resolve a `top_level_structure_locator` to the canonical JSON of the
    /// addressed version (its `uid` injected), verifying EHR ownership.
    async fn resolve_locator(
        &self,
        ehr_id: EhrId,
        locator: &TopLevelLocator,
    ) -> Result<Value, ServiceError> {
        let (vo_id, version) = if let Some(object) = &locator.object {
            // A versioned-object reference: a bare uid assumes the latest trunk
            // version, an exact OBJECT_VERSION_ID selects that version
            // (master11 §"Top-level Structure Locator").
            resolve_object_ref(object)?
        } else {
            let attribute = &locator.attribute;
            // An attribute with no id addresses the EHR's single current object
            // of that kind. `directory` — and a bare `folders`, whose only
            // spec-pinned member is `folders.item(1)` (RM ehr §EHR Class
            // `Directory_in_folders`) — must resolve deterministically among
            // multiple hierarchies, so it takes the rank-ordered lookup.
            let vo_id = match attribute.as_str() {
                "directory" | "folders" => {
                    self.directory_vo_opt(ehr_id).await?.ok_or_else(|| {
                        ServiceError::sm(
                            CallStatusType::VersionedObjectDoesNotExist,
                            format!("{attribute} for EHR {ehr_id}"),
                        )
                    })?
                }
                "ehr_status" | "ehr_access" => {
                    let kind = if attribute == "ehr_status" {
                        Kind::EhrStatus
                    } else {
                        Kind::EhrAccess
                    };
                    self.current_vo(ehr_id, kind)
                        .await?
                        .ok_or_else(|| {
                            ServiceError::sm(
                                CallStatusType::VersionedObjectDoesNotExist,
                                format!("{attribute} for EHR {ehr_id}"),
                            )
                        })?
                        .0
                }
                other => {
                    return Err(ServiceError::precondition(format!(
                        "ehr: locator {other:?} requires a versioned-object id"
                    )));
                }
            };
            (vo_id, None)
        };

        let read = match version {
            Some(v) => read_version(&self.pool, self.spec_profile, vo_id, v).await?,
            None => read_current(&self.pool, self.spec_profile, vo_id).await?,
        }
        .filter(|r| r.ehr_id == Some(ehr_id))
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("versioned object {vo_id} in EHR {ehr_id}"),
            )
        })?;

        if read.deleted() {
            return Err(ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("versioned object {vo_id} is deleted"),
            ));
        }
        Ok(self.version_response(ehr_id, vo_id, read)?.body)
    }
}

/// Decode a versioned-object reference into the storage key pair (`vo_id`,
/// optional exact version). A bare uid → latest trunk (version `None`); an
/// exact `OBJECT_VERSION_ID` → its [`TreeId`].
#[expect(
    clippy::map_err_ignore,
    reason = "the mapped error already names the resource and echoes the \
              rejected token; the discarded `uuid::Error` adds only its own \
              wording, which is not part of the wire contract"
)]
fn resolve_object_ref(object: &VersionLocator) -> Result<(VoId, Option<TreeId>), ServiceError> {
    match object {
        VersionLocator::VersionedObject(uid) => {
            let vo_id = Uuid::parse_str(uid).map_err(|_| {
                ServiceError::precondition(format!("ehr: locator uid {uid:?} is not a UUID"))
            })?;
            // A locator uid names a versioned object.
            Ok((VoId(vo_id), None))
        }
        VersionLocator::Version(ovid) => {
            let (vo_id, tree) = components(ovid)?;
            Ok((vo_id, Some(tree)))
        }
    }
}
