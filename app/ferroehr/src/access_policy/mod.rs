// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's `ACCESS_POLICY` in force on this replica.
//!
//! The policy is the one `VERSIONED_ACCESS_POLICY` of the instance
//! ([`crate::storage::access_policy`]): which kinds of data carry which
//! sensitivity, whether clinical access needs a care relationship and a
//! declared purpose, and the catalogue of named directives a subject accepts
//! by reference. Each replica holds the newest version in memory and listens
//! on [`CHANNEL`]; a write notifies it when it commits, and the next request
//! reads the newest version again ([`LivePolicy::refresh_if_stale`]). While
//! the listener is down every request reads, so a lost notification never
//! leaves an outdated policy in force, and with nothing stale a request
//! touches no database. With no version committed nothing is tightened: no
//! classification, no care relationship or purpose required, an empty
//! catalogue.
//!
//! `ACCESS_POLICY` is a class of FerroEHR's draft access control package and
//! ITS-REST defines no resource for it, so the store, the copy and its
//! invalidation are our own design/extension.

pub mod care;
pub mod check;
pub mod relationship;
pub mod retention;

use crate::db::domain::Domain;
use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use sqlx::PgPool;

use crate::party_binding::grants::live::LiveGrants;
use crate::policy_register::listen::Listened;
use crate::storage::access_policy as store;
use crate::storage::error::StorageError;
use crate::system_log::retention::LogRetention;
use crate::system_log::sender::AuditSender;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;

/// The archetype of the default access policy definition.
pub const ARCHETYPE: &str = "openEHR-EHR-ACCESS_POLICY.controller.v1";

/// The default definition of the access policy archetype, which every boot
/// stores when no definition of its id is stored
/// ([`crate::service::demographic::default_definitions`]).
pub const DEFINITIONS: [(&str, &str); 1] = [(
    "openEHR-EHR-ACCESS_POLICY.controller.v1.0.0.adls",
    include_str!(
        "../../assets/definitions/access_policy/openEHR-EHR-ACCESS_POLICY.controller.v1.0.0.adls"
    ),
)];

/// The channel a policy write notifies and every replica listens on.
pub const CHANNEL: &str = "ferroehr_access_policy";

/// The access policy in force.
#[derive(Debug, Clone, Default)]
pub struct PolicyInForce {
    /// Its trunk version; `0` when no version is committed.
    pub version: i32,
    /// The uid of the version, `OBJECT_VERSION_ID`; `None` when no version
    /// is committed.
    pub uid: Option<String>,
    /// The policy; `None` when no version is committed, which tightens
    /// nothing.
    pub policy: Option<Arc<AccessPolicy>>,
}

impl PolicyInForce {
    /// The policy's facts about care relationships, each void one at its
    /// strictest default.
    #[must_use]
    pub fn care(&self) -> care::CareFacts<'_> {
        care::CareFacts::of(self.policy.as_deref())
    }
}

/// Why the policy could not be read.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// The policy store did not answer.
    #[error("the access policy store did not answer")]
    Store(#[from] StorageError),
}

/// The access policy in force on this replica, read again whenever it gains a
/// version.
pub struct LivePolicy {
    pool: PgPool,
    in_force: RwLock<Arc<PolicyInForce>>,
    /// Serializes the reads that install a new version.
    installing: tokio::sync::Mutex<()>,
    /// The listener on [`CHANNEL`], which says when the policy in force must
    /// be read again.
    listened: Arc<Listened>,
    /// The audit trail the access log retention in force is handed to.
    audit: RwLock<Option<AuditSender>>,
    /// The policy grants whose federation agreements are judged against the
    /// period of the access log retention in force.
    grants: RwLock<Option<Arc<LiveGrants>>>,
    /// The access log retention of the newest version that read, and the
    /// version whose retention did not.
    retention: RwLock<RetentionInForce>,
    /// The `policy_revision` the policy in force was confirmed at: it holds
    /// every version committed up to it (#6251).
    confirmed: AtomicI64,
}

/// The access log retention a replica holds: the one of the newest policy
/// version that read, and the newest version whose retention did not, while
/// that version is the newest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionInForce {
    /// The retention of the newest version that read.
    pub last_good: LogRetention,
    /// The newest version, when its retention does not read: the trail keeps
    /// recording under [`Self::last_good`] and purges nothing until a version
    /// reads again.
    pub unreadable: Option<i32>,
}

impl RetentionInForce {
    /// The retention once version `version` (`uid`) is installed with
    /// `policy`, or `None` when the stored version does not read at all.
    #[must_use]
    pub fn after(&self, version: i32, uid: Option<&str>, policy: Option<&AccessPolicy>) -> Self {
        let read = policy.ok_or(None).and_then(|policy| {
            retention::log_retention(policy.log_retention.as_ref(), uid).map_err(Some)
        });
        match read {
            Ok(last_good) => Self {
                last_good,
                unreadable: None,
            },
            Err(error) => {
                if let Some(error) = error {
                    tracing::error!(
                        version,
                        %error,
                        "the access log retention of access policy version {version} does not \
                         read; records keep the last retention that read and nothing is purged"
                    );
                } else {
                    tracing::error!(
                        version,
                        "access policy version {version} does not read; records keep the last \
                         retention that read and nothing is purged"
                    );
                }
                Self {
                    last_good: self.last_good.clone(),
                    unreadable: Some(version),
                }
            }
        }
    }
}

impl fmt::Debug for LivePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LivePolicy")
            .field("version", &self.in_force().version)
            .finish_non_exhaustive()
    }
}

impl LivePolicy {
    /// The policy over `pool`, at no version until [`Self::refresh`] reads the
    /// stored one.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            listened: Arc::new(Listened::new(CHANNEL, Domain::Clinical)),
            pool,
            in_force: RwLock::new(Arc::default()),
            installing: tokio::sync::Mutex::new(()),
            audit: RwLock::new(None),
            grants: RwLock::new(None),
            retention: RwLock::default(),
            confirmed: AtomicI64::new(0),
        }
    }

    /// Judge `grants`' federation agreements against the period of the
    /// access log retention in force, now and whenever the policy gains a
    /// version.
    pub fn attach_grants(&self, grants: Arc<LiveGrants>) {
        grants.set_federation(
            (*grants.federation())
                .clone()
                .with_holder(self.log_retention().period),
        );
        *self.grants.write().unwrap_or_else(PoisonError::into_inner) = Some(grants);
    }

    /// Hand the access log retention in force to `sender` now and whenever
    /// the policy gains a version.
    pub fn attach_audit(&self, sender: AuditSender) {
        let retention = self.retention_in_force();
        sender.set_retention(&retention.last_good);
        sender.hold_purge(retention.unreadable.is_some());
        *self.audit.write().unwrap_or_else(PoisonError::into_inner) = Some(sender);
    }

    /// The access log retention of the newest policy version that read; the
    /// default, every record kept without end under Art. 9(2), while no
    /// version declares one.
    #[must_use]
    pub fn log_retention(&self) -> LogRetention {
        self.retention_in_force().last_good
    }

    /// The access log retention this replica holds, with the version whose
    /// retention did not read, if the newest is one.
    #[must_use]
    pub fn retention_in_force(&self) -> RetentionInForce {
        self.retention
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Bring the policy in force up to date whenever a write is notified, so
    /// the audit trail takes up a new retention with no request on this
    /// replica: a detached task that lives as long as the runtime.
    pub fn spawn_refresher(self: &Arc<Self>) {
        let live = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let seen = live.listened.notifications();
                live.listened
                    .notified_after(seen, std::time::Duration::from_hours(1))
                    .await;
                if let Err(error) = live.refresh_if_stale().await {
                    tracing::warn!(%error, "the access policy could not be read; retried");
                }
            }
        });
    }

    /// The policy in force.
    #[must_use]
    pub fn in_force(&self) -> Arc<PolicyInForce> {
        // NOTE: a writer swaps one `Arc` whole, so a poisoned lock holds no torn value.
        self.in_force
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The pool the policy is stored in.
    #[must_use]
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The listener that says when the policy in force must be read again;
    /// [`crate::policy_register::listen::Listeners::spawn`] starts it.
    #[must_use]
    pub fn listened(&self) -> &Arc<Listened> {
        &self.listened
    }

    /// Read the policy when a notification marked it stale or the listener is
    /// down; otherwise the policy in force stands, and the request touches no
    /// database.
    ///
    /// # Errors
    /// As [`Self::refresh`].
    pub async fn refresh_if_stale(&self) -> Result<(), PolicyError> {
        if self.listened.must_read() {
            self.refresh().await?;
        }
        Ok(())
    }

    /// Read the newest stored version and install it when this replica does
    /// not hold it yet: one indexed read when nothing changed.
    ///
    /// # Errors
    /// [`PolicyError::Store`] when the store does not answer; the policy in
    /// force stays.
    pub async fn refresh(&self) -> Result<(), PolicyError> {
        let heads = crate::storage::policy_register::policy_heads(&self.pool).await?;
        self.install(heads.access_policy).await?;
        self.confirmed.fetch_max(heads.revision, Ordering::AcqRel);
        Ok(())
    }

    /// Whether the policy in force holds every version committed up to the
    /// policy revision `revision` the request read (#6251).
    #[must_use]
    pub(crate) fn holds(&self, revision: i64) -> bool {
        revision <= self.confirmed.load(Ordering::Acquire)
    }

    /// Bring the policy in force to `heads.access_policy`, the newest stored
    /// version read with the policy revision at one snapshot
    /// ([`crate::storage::policy_register::policy_heads`]), and mark it
    /// confirmed at that revision, so what a request decides rests on its own
    /// read of the revision and on no notification's delay (#6048, #6251).
    ///
    /// # Errors
    /// As [`Self::refresh`].
    pub(crate) async fn confirm(
        &self,
        heads: &crate::storage::policy_register::PolicyHeads,
    ) -> Result<(), PolicyError> {
        // NOTE: our own design (#6048), the pending notification is taken: `heads` is newer.
        let _notified = self.listened.must_read();
        self.install(heads.access_policy).await?;
        self.confirmed.fetch_max(heads.revision, Ordering::AcqRel);
        Ok(())
    }

    /// Install the newest stored version when `head` is past the one this
    /// replica holds. The policy's versions only grow, so a `head` read
    /// before another caller's never puts an older version back in force.
    async fn install(&self, head: i32) -> Result<(), PolicyError> {
        if head <= self.in_force().version {
            return Ok(());
        }
        let _installing = crate::db::acquire::awaiting(self.installing.lock()).await;
        let Some(row) = store::newest(&self.pool).await? else {
            return Ok(());
        };
        if row.version <= self.in_force().version {
            return Ok(());
        }
        let read = match openehr_its::json::from_canonical_json::<AccessPolicy>(&row.data) {
            Ok(policy) => Some(Arc::new(policy)),
            Err(error) => {
                tracing::error!(
                    version = row.version,
                    error = %error,
                    "a stored access policy version does not read; the policy in force stays"
                );
                None
            }
        };
        let uid = crate::versioning::object_version_id::object_version_id(
            crate::ids::VoId(row.vo_id),
            &row.system_id,
            crate::versioning::object_version_id::TreeId::trunk(row.version),
        );
        let retention = self
            .retention_in_force()
            .after(row.version, Some(&uid), read.as_deref());
        let installed = Arc::new(PolicyInForce {
            version: row.version,
            uid: Some(uid),
            policy: read.or_else(|| self.in_force().policy.clone()),
        });
        *self
            .in_force
            .write()
            .unwrap_or_else(PoisonError::into_inner) = installed;
        let audit = self
            .audit
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        *self
            .retention
            .write()
            .unwrap_or_else(PoisonError::into_inner) = retention.clone();
        if let Some(sender) = audit {
            sender.set_retention(&retention.last_good);
            sender.hold_purge(retention.unreadable.is_some());
        }
        let grants = self
            .grants
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(grants) = grants {
            grants.set_federation(
                (*grants.federation())
                    .clone()
                    .with_holder(retention.last_good.period),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_log::config::Retention;

    /// A policy whose log retention keeps entries `period`, in the Union.
    fn policy(period: &str) -> AccessPolicy {
        let text = format!(
            r#"{{"_type": "ACCESS_POLICY",
            "name": {{"_type": "DV_TEXT", "value": "Access policy"}},
            "archetype_node_id": "{ARCHETYPE}",
            "controller": {{"_type": "PARTY_REF", "namespace": "demographic",
                "type": "ORGANISATION",
                "id": {{"_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7"}}}},
            "care_relationship_required": false,
            "purpose_required": false,
            "log_retention": {{"_type": "ACCESS_LOG_RETENTION",
                "name": {{"_type": "DV_TEXT", "value": "Access log retention"}},
                "archetype_node_id": "id18",
                "in_union": true,
                "period": {{"_type": "DV_DURATION", "value": "{period}"}}}}}}"#
        );
        openehr_its::json::from_canonical_json(&text).expect("a canonical ACCESS_POLICY")
    }

    /// A version whose retention reads replaces the one in force; one whose
    /// retention does not read, or that does not read at all, keeps the last
    /// retention that read and is named until a version reads again.
    #[test]
    fn an_unreadable_retention_keeps_the_last_good_one_and_is_named() {
        let first = RetentionInForce::default().after(1, Some("uid::cdr::1"), Some(&policy("P5Y")));
        assert_eq!(first.unreadable, None);
        assert_eq!(first.last_good.period, Retention::Years(5));
        assert_eq!(first.last_good.policy.as_deref(), Some("uid::cdr::1"));

        let months = first.after(2, Some("uid::cdr::2"), Some(&policy("P18M")));
        assert_eq!(
            months.unreadable,
            Some(2),
            "the version that did not read is named"
        );
        assert_eq!(
            months.last_good, first.last_good,
            "the last good retention stays"
        );

        let unread = months.after(3, Some("uid::cdr::3"), None);
        assert_eq!(unread.unreadable, Some(3));
        assert_eq!(unread.last_good, first.last_good);

        let again = unread.after(4, Some("uid::cdr::4"), Some(&policy("P6Y")));
        assert_eq!(again.unreadable, None);
        assert_eq!(again.last_good.period, Retention::Years(6));
    }
}
