// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's `ACCESS_POLICY` behind its extension resource.
//!
//! The service reads the version in force, every version and the
//! `VERSIONED_ACCESS_POLICY`, writes a new version, and brings the copy in
//! force up to date per request.
//!
//! The policy is one versioned object. Each write commits one
//! `ORIGINAL_VERSION` on its trunk in its own `CONTRIBUTION`, with the
//! `AUDIT_DETAILS` of the request's committer (RM Common IM master06
//! §Contributions, §Committal and Audits). A write names the version it
//! replaces, or creates the first one, so a writer that read an older
//! version is refused (`412`) rather than overwriting a change it never saw.
//! A body that does not hold is refused whole (`400`) and never reaches the
//! request path. The draft access control package is no openEHR release and
//! ITS-REST defines no resource for the policy: our own design/extension.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the version envelope is served as the \
              canonical JSON its stored fragments already are"
)]

use std::sync::Arc;

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
use serde_json::Value;
use sqlx::Connection as _;
use uuid::Uuid;

use crate::access_policy::check::checked;
use crate::access_policy::draft::Policy;
use crate::access_policy::{CHANNEL, LivePolicy};
use crate::db::acquire::acquire;
use crate::error::{ErrorChain, ServiceError};
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::storage::access_policy::{self as store, NewVersion, PolicyRow};
use crate::versioning::audit::change_type;
use crate::versioning::lifecycle::state::COMPLETE;
use crate::versioning::object_version_id::{TreeId, object_version_id, parse_object_version_id};
use crate::versioning::wire::{OriginalVersionParts, build_original_version, contribution_ref};

/// The `AUDIT_DETAILS.description` of a policy version.
const DESCRIPTION: &str = "access policy written through the access policy resource";

/// What a write expects of the stored policy (RFC 9110 §13.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyPrecondition {
    /// No version is committed yet (`If-None-Match: *`): the write creates
    /// the policy.
    Create,
    /// The newest version is the one this `OBJECT_VERSION_ID` names
    /// (`If-Match`): the write replaces it.
    Replace(String),
}

/// One version of the policy as the resource serves it.
#[derive(Debug, Clone)]
pub struct PolicyVersion {
    /// The `OBJECT_VERSION_ID` of the version, its `ETag`.
    pub version_uid: String,
    /// When the version was committed, its `Last-Modified`.
    pub time_committed: jiff::Timestamp,
    /// The canonical `ACCESS_POLICY`.
    pub data: Value,
    /// The canonical `ORIGINAL_VERSION` with its contribution and commit
    /// audit.
    pub original_version: Value,
}

impl PolicyVersion {
    /// A stored version, with `preceding` the uid of the version before it.
    fn stored(row: &PolicyRow, preceding: Option<&str>) -> Result<Self, ServiceError> {
        let vo_id = VoId(row.vo_id);
        let tree = TreeId::trunk(row.version);
        let data: Value = serde_json::from_str(&row.data)?;
        let commit_audit: Value = serde_json::from_str(&row.commit_audit)?;
        let original_version = build_original_version(&OriginalVersionParts {
            creating_system_id: &row.system_id,
            vo_id,
            tree,
            preceding_version_uid: preceding,
            other_input_version_uids: &[],
            contribution: &contribution_ref(row.contribution_id),
            commit_audit: &commit_audit,
            lifecycle_state: &row.lifecycle_state,
            data: &data,
            attestations: &[],
            signature: None,
        })
        .map_err(|error| ServiceError::internal("render a stored access policy version", error))?;
        Ok(Self {
            version_uid: object_version_id(vo_id, &row.system_id, tree),
            time_committed: row.time_committed,
            data,
            original_version,
        })
    }
}

/// The versioned object of the policy as the resource serves it.
#[derive(Debug, Clone)]
pub struct VersionedPolicy {
    /// `VERSIONED_ACCESS_POLICY.uid`, its `ETag`.
    pub uid: String,
    /// The canonical `VERSIONED_ACCESS_POLICY`.
    pub body: Value,
    /// The commit time of the newest version, its `Last-Modified`.
    pub last_modified: jiff::Timestamp,
}

/// The uid of the version before `row`, which sits one trunk version down on
/// the same object and system.
fn preceding_uid(row: &PolicyRow, before: Option<&PolicyRow>) -> Option<String> {
    before
        .map(|b| object_version_id(VoId(b.vo_id), &b.system_id, TreeId::trunk(b.version)))
        .filter(|_| row.version > 1)
}

impl FerroEhrService {
    /// The access policy in force on this replica.
    #[must_use]
    pub fn access_policy(&self) -> &Arc<LivePolicy> {
        &self.access_policy
    }

    /// Bring the access policy in force up to date before the request path
    /// decides anything: a read when a policy write was notified or the
    /// listener is down, and no database work otherwise.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the policy store does not answer.
    pub async fn refresh_access_policy(&self) -> Result<(), ServiceError> {
        self.access_policy
            .refresh_if_stale()
            .await
            .map_err(|error| ServiceError::overloaded("the access policy could not be read", error))
    }

    /// Read the newest version and install it when this replica does not hold
    /// it, whatever the listener says.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the policy store does not answer.
    pub async fn read_access_policy(&self) -> Result<(), ServiceError> {
        self.access_policy
            .refresh()
            .await
            .map_err(|error| ServiceError::overloaded("the access policy could not be read", error))
    }

    /// The newest version of the policy, or `None` when none is committed.
    ///
    /// # Errors
    /// `exception` on a database fault or a stored version that does not
    /// render.
    pub async fn access_policy_current(&self) -> Result<Option<PolicyVersion>, ServiceError> {
        let pool = self.access_policy.pool();
        let Some(row) = store::newest(pool).await? else {
            return Ok(None);
        };
        let before = match row.version.checked_sub(1) {
            Some(version) if version > 0 => store::at(pool, version).await?,
            _ => None,
        };
        let preceding = preceding_uid(&row, before.as_ref());
        PolicyVersion::stored(&row, preceding.as_deref()).map(Some)
    }

    /// Every version of the policy, newest first; empty when none is
    /// committed.
    ///
    /// # Errors
    /// `exception` on a database fault or a stored version that does not
    /// render.
    pub async fn access_policy_history(&self) -> Result<Vec<PolicyVersion>, ServiceError> {
        let rows = store::history(self.access_policy.pool()).await?;
        rows.iter()
            .enumerate()
            .map(|(at, row)| {
                let preceding = preceding_uid(row, rows.get(at.saturating_add(1)));
                PolicyVersion::stored(row, preceding.as_deref())
            })
            .collect()
    }

    /// The `VERSIONED_ACCESS_POLICY`, or `None` when no version is committed.
    ///
    /// Its `owner_id` is the controller the first version names, and
    /// `time_created` that version's commit time (RM Common IM,
    /// `VERSIONED_OBJECT`).
    ///
    /// # Errors
    /// `exception` on a database fault or a stored version that does not
    /// read.
    pub async fn access_policy_versioned(&self) -> Result<Option<VersionedPolicy>, ServiceError> {
        let pool = self.access_policy.pool();
        let Some(newest) = store::newest(pool).await? else {
            return Ok(None);
        };
        let Some(first) = store::at(pool, 1).await? else {
            return Ok(None);
        };
        let owner = openehr_its::json::from_canonical_json::<Policy>(&first.data)
            .map_err(|error| ServiceError::internal("read the first access policy version", error))?
            .controller;
        let versioned = crate::access_policy::draft::VersionedPolicy {
            uid: HierObjectId::from(first.vo_id),
            owner_id: ObjectRef::PartyRef(owner),
            time_created: crate::versioning::audit::dv_date_time(&first.time_committed),
        };
        Ok(Some(VersionedPolicy {
            uid: first.vo_id.to_string(),
            body: openehr_its::json::to_canonical_value(&versioned),
            last_modified: newest.time_committed,
        }))
    }

    /// Commit `body` as the next version of the policy when `precondition`
    /// holds, and install it on this replica; the version written.
    ///
    /// # Errors
    /// - `bad_request` (`400`) — the body is no canonical-JSON
    ///   `ACCESS_POLICY`, breaks a draft BMM invariant, its root is no
    ///   `ACCESS_POLICY` archetype root, its controller is no `ORGANISATION`
    ///   or another one than the policy's; or `If-Match` is no
    ///   `OBJECT_VERSION_ID`.
    /// - `version_mismatch` (`412`) — `precondition` does not hold.
    /// - `service_overloaded` (`503`) — the policy store does not answer.
    /// - `exception` — a database fault (rolled back).
    pub async fn put_access_policy(
        &self,
        precondition: &PolicyPrecondition,
        body: &Value,
    ) -> Result<PolicyVersion, ServiceError> {
        let policy = checked(body).map_err(|defect| {
            ServiceError::bad_request(
                format!("the access policy is refused: {}", ErrorChain::new(&defect)),
                defect,
            )
        })?;
        let mut conn = acquire(self.access_policy.pool()).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        let newest = store::lock_newest(&mut tx).await?;
        let (vo_id, version, change) = match (precondition, &newest) {
            (PolicyPrecondition::Create, None) => (self.stamp.mint(), 1, change_type::CREATION),
            (PolicyPrecondition::Create, Some(row)) => {
                return Err(ServiceError::version_conflict(format!(
                    "an access policy is committed at version {}: name it in If-Match to \
                     replace it",
                    object_version_id(VoId(row.vo_id), &row.system_id, TreeId::trunk(row.version))
                )));
            }
            (PolicyPrecondition::Replace(_), None) => {
                return Err(ServiceError::version_conflict(
                    "no access policy is committed: create it with If-None-Match: *",
                ));
            }
            (PolicyPrecondition::Replace(uid), Some(row)) => {
                replaces(uid, row)?;
                let owner = openehr_its::json::from_canonical_json::<Policy>(&row.data).map_err(
                    |error| ServiceError::internal("read the access policy in force", error),
                )?;
                if owner.controller != policy.controller {
                    return Err(ServiceError::precondition(
                        "the access policy is refused: its controller is not the controller the \
                         policy belongs to",
                    ));
                }
                let Some(next) = row.version.checked_add(1) else {
                    return Err(ServiceError::version_conflict(
                        "the access policy has no further trunk version",
                    ));
                };
                (row.vo_id, next, change_type::MODIFICATION)
            }
        };
        let system_id = self.effective_system_id();
        let time_committed = crate::storage::version_repo::placement::tx_now(&mut tx).await?;
        let commit_audit = openehr_its::json::to_canonical_json(
            &self.audit(change, DESCRIPTION).typed(&time_committed),
        );
        let data = openehr_its::json::to_canonical_json(&policy);
        let contribution_id: Uuid = self.stamp.mint();
        store::append(
            &mut tx,
            version,
            &NewVersion {
                vo_id,
                system_id: &system_id,
                contribution_id,
                commit_audit: &commit_audit,
                time_committed,
                lifecycle_state: COMPLETE,
                data: &data,
            },
        )
        .await?;
        // NOTE: PostgreSQL 18 NOTIFY (sql-notify) delivers at commit, to every listening replica.
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(CHANNEL)
            .bind(version.to_string())
            .execute(&mut *tx)
            .await
            .map_err(ServiceError::Database)?;
        tx.commit().await.map_err(ServiceError::Database)?;
        self.read_access_policy().await?;
        self.access_policy_current().await?.ok_or_else(|| {
            ServiceError::exception("the access policy version just committed does not read")
        })
    }
}

/// Refuse a write whose `If-Match` does not name `row`, the newest version.
fn replaces(uid: &str, row: &PolicyRow) -> Result<(), ServiceError> {
    let (vo_id, system_id, tree) = parse_object_version_id(uid).map_err(|error| {
        ServiceError::bad_request(
            format!("If-Match {uid:?} is not an OBJECT_VERSION_ID"),
            error,
        )
    })?;
    if vo_id != row.vo_id || system_id != row.system_id || tree != TreeId::trunk(row.version) {
        return Err(ServiceError::version_conflict(format!(
            "the access policy is no longer at version {uid}: read it again and write the change \
             over the newest version"
        )));
    }
    Ok(())
}
