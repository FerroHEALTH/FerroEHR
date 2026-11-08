// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A write of a SCIM Group after its creation: a rename, a change of its
//! `externalId` or its members, each one new version of its `GROUP`, and its
//! logical deletion; and the memberships a deleted User leaves.
//!
//! "A Version of a `PARTY` includes all the compositional parts, such as
//! identities, contacts, Party relationships of which it is the source" (RM
//! demographic `master02-demographic_package.adoc` §Versioning Semantics), so
//! every membership change is a new version of the `GROUP` and the member's
//! `PERSON` keeps its version. A Group's `GROUP` in `inactive` or `abandoned`
//! is written only by an administrator: the state machine of RM common
//! `master06-change_control_package.adoc` §Version Lifecycle draws no update of
//! either state, and a SCIM Group has no `active` to reactivate it with. No
//! openEHR spec governs SCIM: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use serde_json::Value;

use crate::db::acquire::acquire;
use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
use crate::party_binding::provisioned;
use crate::service::FerroEhrService;
use crate::service::demographic::group_provision::{
    GroupState, GroupWriter, ProvisionedGroup, distinct, is_membership, members_of, service_error,
    target_of, too_many,
};
use crate::service::demographic::template::TemplateFit;
use crate::service::demographic::types::PartyKind;
use crate::service::demographic::user_write::ScimCommit;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{Change, WriteEnvelope};
use crate::versioning::lifecycle::state;

impl FerroEhrService {
    /// Writes `state` to the Group `id` of the client of `writer` as one new
    /// version of its `GROUP`, returning the Group as it stands after, or
    /// `None` when the id names no Group of that client.
    ///
    /// A write that changes nothing commits nothing. Only the members it
    /// adds are checked, so a Group keeps a member whose `PERSON` changed
    /// after it joined.
    ///
    /// # Errors
    /// [`ServiceError::VersionConflict`] when the current version is not
    /// `expected`; a `422` naming `active` for a Group that is not active, and
    /// naming `members` for an added member that is no User of the client or
    /// an add past `max_members`; the `409` of a `displayName` or `externalId`
    /// another Group holds; the commit refusals; a storage failure.
    pub async fn update_group(
        &self,
        writer: GroupWriter<'_>,
        id: VoId,
        state: GroupState,
        expected: Option<&str>,
    ) -> Result<Option<ProvisionedGroup>, ServiceError> {
        let mapping = self.scim_group_mapping()?;
        let Some(group) = self.provisioned_group(writer.client, id).await? else {
            return Ok(None);
        };
        expect_version(&group, expected)?;
        if !matches!(
            group.row.lifecycle_state.as_str(),
            state::COMPLETE | state::INCOMPLETE
        ) {
            return Err(ServiceError::content_invalid(
                Violation::new(
                    "the Group is not active: only an administrator writes an inactive or \
                     abandoned GROUP",
                )
                .with_path("active"),
            ));
        }
        let members = distinct(&state.members);
        if members.len() > writer.max_members && members.len() > group.members.len() {
            return Err(too_many(writer.max_members));
        }
        let added: Vec<VoId> = members
            .iter()
            .copied()
            .filter(|member| !group.members.contains(member))
            .collect();
        self.require_users(writer.client.binding_issuer, &added)
            .await?;
        let body = self
            .group_body(mapping, writer, Some(&group.body), (id, &state, &members))
            .await?;
        if body == group.body {
            return Ok(Some(group));
        }
        let lifecycle = match self.party_template_fit(&body, mapping.template).await? {
            TemplateFit::Complete => state::COMPLETE,
            TemplateFit::Incomplete(_) => state::INCOMPLETE,
        };
        let mut written = body.clone();
        let change = Change::modify(
            id,
            Kind::Group,
            body,
            Some(self.group_tree(id).await?),
            None,
            WriteEnvelope {
                lifecycle_state: Some(lifecycle.to_owned()),
                ..WriteEnvelope::default()
            },
        );
        let commit = ScimCommit::new(writer.client.binding_issuer, change_type::MODIFICATION);
        let committed = self
            .commit_scim(commit, vec![change])
            .await
            .map_err(service_error)?;
        let version = committed.versions.into_iter().next().ok_or_else(|| {
            ServiceError::exception(format!("the change of the GROUP {id} committed no version"))
        })?;
        // The Group as this commit wrote it: reading it back would parse a
        // body as large as the Group again (#5220).
        let version_uid = version.version_uid();
        crate::versioning::change::stamp_version_uid(&mut written, &version_uid)
            .map_err(|e| ServiceError::internal("stamp the GROUP version uid", e))?;
        Ok(Some(ProvisionedGroup {
            row: provisioned::GroupRow {
                party: id,
                display_name: state.display_name,
                lifecycle_state: lifecycle.to_owned(),
                external_id: state.external_id,
            },
            version_uid,
            created: group.created,
            last_modified: version.time_committed,
            members: members_of(&written, mapping.membership),
            body: written,
        }))
    }

    /// Logically deletes the Group `id` of the client of `writer`: its
    /// `GROUP`, with the memberships it holds, in one demographic
    /// CONTRIBUTION (RM common `master06-change_control_package.adoc`
    /// §Logical Deletion). `false` when the id names no Group of that client.
    ///
    /// # Errors
    /// [`ServiceError::VersionConflict`] when the current version is not
    /// `expected`; the commit refusals; a storage failure.
    pub async fn delete_group(
        &self,
        writer: GroupWriter<'_>,
        id: VoId,
        expected: Option<&str>,
    ) -> Result<bool, ServiceError> {
        let Some(group) = self.provisioned_group(writer.client, id).await? else {
            return Ok(false);
        };
        expect_version(&group, expected)?;
        let change = Change::delete(id, Kind::Group, Some(self.group_tree(id).await?));
        let commit = ScimCommit::new(writer.client.binding_issuer, change_type::DELETED);
        self.commit_scim(commit, vec![change])
            .await
            .map_err(service_error)?;
        Ok(true)
    }

    /// The new version of each live `GROUP` that holds a membership of the
    /// `PERSON` `member`, without it, for the CONTRIBUTION that deletes the
    /// `PERSON`.
    ///
    /// A `GROUP` in `inactive` or `abandoned` keeps its version: no transition
    /// the state machine draws re-commits it without reopening it, and such a
    /// `GROUP` lists no member to the binding.
    pub(super) async fn membership_removals(
        &self,
        member: VoId,
    ) -> Result<Vec<Change>, ServiceError> {
        let Ok(mapping) = self.scim_group_mapping() else {
            return Ok(Vec::new());
        };
        let groups = {
            let mut conn = acquire(&self.demographic_pool).await?;
            provisioned::groups_of_member(&mut conn, member).await?
        };
        let mut changes = Vec::with_capacity(groups.len());
        for group in groups {
            let Some(current) = self.party_current(PartyKind::Group, group).await? else {
                continue;
            };
            if current.deleted {
                continue;
            }
            let read = self.read_party(PartyKind::Group, group, None, None).await?;
            let lifecycle = self.head_lifecycle(group).await?;
            if !matches!(lifecycle.as_str(), state::COMPLETE | state::INCOMPLETE) {
                continue;
            }
            let mut body = read.body;
            let Some(object) = body.as_object_mut() else {
                continue;
            };
            let kept: Vec<Value> = object
                .get("relationships")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter(|relationship| {
                    !(is_membership(relationship, mapping.membership)
                        && target_of(relationship) == Some(member))
                })
                .cloned()
                .collect();
            if kept.is_empty() {
                object.remove("relationships");
            } else {
                object.insert("relationships".to_owned(), Value::Array(kept));
            }
            changes.push(Change::modify(
                group,
                Kind::Group,
                body,
                Some(current.tree),
                None,
                WriteEnvelope {
                    lifecycle_state: Some(lifecycle),
                    ..WriteEnvelope::default()
                },
            ));
        }
        Ok(changes)
    }

    /// The tree id of the head of the `GROUP` `id`.
    async fn group_tree(
        &self,
        id: VoId,
    ) -> Result<crate::versioning::object_version_id::TreeId, ServiceError> {
        Ok(self
            .party_current(PartyKind::Group, id)
            .await?
            .ok_or_else(|| ServiceError::exception(format!("the GROUP {id} has no head")))?
            .tree)
    }

    /// The `version_lifecycle_state` code of the head of the party `id`.
    async fn head_lifecycle(&self, id: VoId) -> Result<String, ServiceError> {
        crate::storage::version_repo::meta::current_demographic_meta(&self.demographic_pool, id)
            .await?
            .map(|meta| meta.lifecycle_state)
            .ok_or_else(|| ServiceError::exception(format!("the party {id} has no head")))
    }
}

/// Refuses a write naming a version of `group` other than the current one.
fn expect_version(group: &ProvisionedGroup, expected: Option<&str>) -> Result<(), ServiceError> {
    match expected {
        Some(expected) if expected != group.version_uid => Err(ServiceError::version_conflict(
            format!("the Group's version is {}", group.version_uid),
        )),
        _ => Ok(()),
    }
}
