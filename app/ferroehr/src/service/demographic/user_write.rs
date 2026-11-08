// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A write of a SCIM User after its creation: the next version of its
//! `PERSON`, from the head with only the mapped paths replaced or cleared, in
//! the lifecycle state [`super::user_lifecycle::transition`] gives.
//!
//! Each request commits one version (two for an `abandoned` User completed in
//! one request, RM common `master06-change_control_package.adoc` §Version
//! Lifecycle), and an `If-Match` that names an older version is refused
//! before anything is written (RFC 7644 §3.14). No openEHR spec governs SCIM:
//! our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use serde_json::Value;
use sqlx::Connection as _;

use crate::db::acquire::acquire;
use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
use crate::party_binding::provisioned;
use crate::service::FerroEhrService;
use crate::service::demographic::mapped_write::clear_mapped_value;
use crate::service::demographic::provision::ProvisionedUser;
use crate::service::demographic::template::TemplateFit;
use crate::service::demographic::types::PartyKind;
use crate::service::demographic::user_lifecycle::{Transition, transition};
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{self, Change, CommittedContribution, WriteEnvelope};
use crate::versioning::contribution::typed_decode_gate;
use crate::versioning::lifecycle::state;

/// What a write changes in a User.
#[derive(Debug, Clone, Default)]
pub struct UserUpdate {
    /// The data value each mapped path takes.
    pub set: Vec<(String, Value)>,
    /// The mapped paths cleared, each only the identifier of its `type` and
    /// `issuer` when given.
    pub clear: Vec<(String, Option<(String, String)>)>,
    /// The request's `active`, `None` when it leaves it out.
    pub active: Option<bool>,
    /// The version the request names as current (`If-Match`).
    pub expected: Option<String>,
}

/// The bound on the Users one client deactivates or deletes per window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deactivations {
    /// The most deactivations and deletions in one window.
    pub max: u32,
    /// The window, in minutes.
    pub window_minutes: u32,
}

/// Why a User write fails.
#[derive(Debug, thiserror::Error)]
pub enum UserWriteError {
    /// The demographic service refused it or failed.
    #[error(transparent)]
    Service(#[from] ServiceError),
    /// The client already deactivated or deleted the most Users its window
    /// holds.
    #[error("the SCIM client deactivated or deleted {max} Users within {window_minutes} minutes")]
    Limited {
        /// The bound.
        max: u32,
        /// The window, in minutes.
        window_minutes: u32,
    },
}

impl From<crate::storage::error::StorageError> for UserWriteError {
    fn from(error: crate::storage::error::StorageError) -> Self {
        Self::Service(error.into())
    }
}

impl From<sqlx::Error> for UserWriteError {
    fn from(error: sqlx::Error) -> Self {
        Self::Service(error.into())
    }
}

impl FerroEhrService {
    /// Writes `update` to the User `id` of the client of `binding_issuer`,
    /// returning the User as it stands after, or `None` when the id names no
    /// User of that client.
    ///
    /// # Errors
    /// [`ServiceError::VersionConflict`] when the current version is not the
    /// one `update` names; the `422` of an attribute change
    /// to an inactive or abandoned User that does not reactivate it, and of a
    /// body that does not fit its template even incomplete; the commit
    /// refusals, a protected identifier dropped or a key changed among them;
    /// a storage failure; [`UserWriteError::Limited`] for a deactivation past
    /// `limit`.
    pub async fn update_user(
        &self,
        binding_issuer: &str,
        id: VoId,
        update: UserUpdate,
        limit: Deactivations,
    ) -> Result<Option<ProvisionedUser>, UserWriteError> {
        let Some(user) = self.provisioned_user(binding_issuer, id).await? else {
            return Ok(None);
        };
        expect_version(&user, update.expected.as_deref())?;
        let template = user
            .body
            .pointer("/archetype_details/template_id/value")
            .and_then(Value::as_str)
            .ok_or_else(|| ServiceError::exception(format!("the PERSON {id} names no template")))?
            .to_owned();
        let mut body = self
            .mapped_party_body(Some(&user.body), &template, &update.set)
            .await?;
        for (path, identifier) in &update.clear {
            let identifier = identifier.as_ref().map(|(t, i)| (t.as_str(), i.as_str()));
            clear_mapped_value(&mut body, path, identifier);
        }
        let changed = body != user.body;
        let fits = matches!(
            self.party_template_fit(&body, &template).await?,
            TemplateFit::Complete
        );
        let outcome = transition(&user.row.lifecycle_state, update.active, changed, fits).map_err(
            |refused| {
                ServiceError::content_invalid(
                    Violation::new(refused.to_string()).with_path("active"),
                )
            },
        )?;
        let commits: &[&str] = match outcome {
            Transition::Unchanged => return Ok(Some(user)),
            Transition::One(lifecycle) => &[lifecycle][..],
            Transition::RetrieveThenComplete => &[state::INCOMPLETE, state::COMPLETE][..],
        };
        for lifecycle in commits {
            // NOTE: RM common master06 §Abandoned and Inactive States, an abandoned
            // item is one that lost relevance while incomplete, so its content is.
            let incomplete = matches!(*lifecycle, state::INCOMPLETE | state::ABANDONED);
            typed_decode_gate(Kind::Person, &body, incomplete)?;
            self.validate_for_commit(Kind::Person, &body, incomplete)
                .await?;
            let tree = self.head_tree(id).await?;
            let envelope = WriteEnvelope {
                lifecycle_state: Some((*lifecycle).to_owned()),
                ..WriteEnvelope::default()
            };
            let change = Change::modify(id, Kind::Person, body.clone(), Some(tree), None, envelope);
            let deactivates = matches!(*lifecycle, state::INACTIVE | state::ABANDONED);
            let commit = ScimCommit {
                limit: deactivates.then_some(limit),
                ..ScimCommit::new(binding_issuer, change_type::MODIFICATION)
            };
            self.commit_scim(commit, vec![change]).await?;
        }
        Ok(self.provisioned_user(binding_issuer, id).await?)
    }

    /// Logically deletes the User `id` of the client of `binding_issuer`:
    /// its `PERSON` and its professional `ROLE`s, with its memberships taken
    /// out of every live `GROUP`, in one demographic CONTRIBUTION (RM common
    /// `master06-change_control_package.adoc` §Logical Deletion). `false` when the id names no User of that client.
    /// With `contact_erasure_minutes` zero, the same transaction erases the
    /// contacts of every version of the `PERSON`; otherwise the sweep
    /// ([`FerroEhrService::erase_due_contacts`]) erases them that long after.
    ///
    /// # Errors
    /// [`ServiceError::VersionConflict`] when the current version is not
    /// `expected`; [`UserWriteError::Limited`] past `limit`; the commit
    /// refusals; a storage failure.
    pub async fn delete_user(
        &self,
        binding_issuer: &str,
        id: VoId,
        expected: Option<&str>,
        limit: Deactivations,
        contact_erasure_minutes: u32,
    ) -> Result<bool, UserWriteError> {
        let Some(user) = self.provisioned_user(binding_issuer, id).await? else {
            return Ok(false);
        };
        expect_version(&user, expected)?;
        let mut changes = vec![Change::delete(
            id,
            Kind::Person,
            Some(self.head_tree(id).await?),
        )];
        for role in self.professional_roles(&user.body).await? {
            changes.push(Change::delete(role, Kind::Role, None));
        }
        changes.extend(self.membership_removals(id).await?);
        let commit = ScimCommit {
            limit: Some(limit),
            erase: (contact_erasure_minutes == 0).then_some(id),
            ..ScimCommit::new(binding_issuer, change_type::DELETED)
        };
        self.commit_scim(commit, changes).await?;
        Ok(true)
    }

    /// The live professional `ROLE`s the `PERSON` `body` lists in `roles`.
    async fn professional_roles(&self, body: &Value) -> Result<Vec<VoId>, ServiceError> {
        let mut roles = Vec::new();
        let listed = body
            .get("roles")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for reference in listed {
            // NOTE: a reference whose root is no UUID names no container of this store.
            let Some(role) = reference
                .pointer("/id/value")
                .and_then(Value::as_str)
                .and_then(|value| VoId::from_uid_head(value).ok())
            else {
                continue;
            };
            let Some(current) = self.party_current(PartyKind::Role, role).await? else {
                continue;
            };
            if current.deleted {
                continue;
            }
            let read = self.read_party(PartyKind::Role, role, None, None).await?;
            let template = read
                .body
                .pointer("/archetype_details/template_id/value")
                .and_then(Value::as_str);
            if template.is_some_and(|template| self.binding.is_professional_role(template)) {
                roles.push(role);
            }
        }
        Ok(roles)
    }

    /// The tree id of the head of the `PERSON` `id`.
    async fn head_tree(
        &self,
        id: VoId,
    ) -> Result<crate::versioning::object_version_id::TreeId, ServiceError> {
        Ok(self
            .party_current(PartyKind::Person, id)
            .await?
            .ok_or_else(|| ServiceError::exception(format!("the PERSON {id} has no head")))?
            .tree)
    }

    /// Commits `changes` in one demographic CONTRIBUTION as `commit` names
    /// it: of its change type, as the SCIM client of its `binding_issuer`,
    /// counted against its `limit` in the same transaction when it
    /// deactivates or deletes a User, recording the client as the owner of
    /// the `GROUP` it creates, and erasing the contacts of the `PERSON` it
    /// names before it commits.
    pub(super) async fn commit_scim(
        &self,
        commit: ScimCommit<'_>,
        changes: Vec<Change>,
    ) -> Result<CommittedContribution, UserWriteError> {
        let ScimCommit {
            binding_issuer,
            limit,
            change_type: change,
            erase,
            created_group,
        } = commit;
        let audit = self.audit(change, "SCIM provisioning");
        let changes = changes
            .into_iter()
            .map(|change| (audit.clone(), change))
            .collect();
        let mut conn = acquire(&self.demographic_pool)
            .await
            .map_err(ServiceError::from)?;
        let mut tx = conn.begin().await?;
        if let Some(limit) = limit
            && !provisioned::count_deactivation(
                &mut tx,
                binding_issuer,
                limit.max,
                limit.window_minutes,
            )
            .await?
        {
            tracing::error!(
                max = limit.max,
                window_minutes = limit.window_minutes,
                "a SCIM client reached max_deactivations: its deactivations and deletions are \
                 refused until the window ends"
            );
            return Err(UserWriteError::Limited {
                max: limit.max,
                window_minutes: limit.window_minutes,
            });
        }
        if let Some(group) = created_group {
            provisioned::record_group_owner(&mut tx, binding_issuer, group).await?;
        }
        let committed = crate::party_binding::protect::provisioning(
            binding_issuer.to_owned(),
            change::commit_contribution(
                &mut tx,
                None,
                None,
                &audit,
                changes,
                Vec::new(),
                &self.signing_ctx(),
            ),
        )
        .await?;
        if let Some(party) = erase {
            self.erase_contacts_in(&mut tx, party).await?;
        }
        tx.commit().await?;
        Ok(committed)
    }
}

/// How [`FerroEhrService::commit_scim`] commits a SCIM client's change-set.
#[derive(Debug, Clone, Copy)]
pub(super) struct ScimCommit<'a> {
    /// The client's `binding_issuer`.
    pub(super) binding_issuer: &'a str,
    /// The bound a deactivation or deletion counts against.
    pub(super) limit: Option<Deactivations>,
    /// The CONTRIBUTION's change type code.
    pub(super) change_type: &'a str,
    /// The `PERSON` whose contacts are erased in the same transaction.
    pub(super) erase: Option<VoId>,
    /// The `GROUP` the change-set creates, recorded as the client's.
    pub(super) created_group: Option<VoId>,
}

impl<'a> ScimCommit<'a> {
    /// A commit of `change_type` by the client of `binding_issuer`, with no
    /// bound, no erasure and no `GROUP` created.
    pub(super) const fn new(binding_issuer: &'a str, change_type: &'a str) -> Self {
        Self {
            binding_issuer,
            limit: None,
            change_type,
            erase: None,
            created_group: None,
        }
    }
}

/// Refuses a write naming a version of `user` other than the current one.
fn expect_version(user: &ProvisionedUser, expected: Option<&str>) -> Result<(), ServiceError> {
    match expected {
        Some(expected) if expected != user.version_uid => Err(ServiceError::version_conflict(
            format!("the User's version is {}", user.version_uid),
        )),
        _ => Ok(()),
    }
}
