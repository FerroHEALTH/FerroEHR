// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A write of a SCIM Group after its creation, and its logical deletion.
//!
//! A rename or a change of its `externalId` is one new version of its
//! `GROUP`, and a change of its members one new version of each member's
//! `PERSON`.
//!
//! "A Version of a `PARTY` includes all the compositional parts, such as
//! identities, contacts, Party relationships of which it is the source" (RM
//! demographic `master02-demographic_package.adoc` §Versioning Semantics),
//! and the member is the source of its membership, so the `GROUP` keeps its
//! version when its members change (#5220). A Group's `GROUP` in `inactive`
//! or `abandoned` is written only by an administrator: the state machine of
//! RM common `master06-change_control_package.adoc` §Version Lifecycle draws
//! no update of either state, and a SCIM Group has no `active` to reactivate
//! it with. No openEHR spec governs SCIM: our own design.

use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::group_provision::{
    GroupState, GroupWriter, ProvisionedGroup, distinct, service_error, too_many,
};
use crate::service::demographic::template::TemplateFit;
use crate::service::demographic::types::PartyKind;
use crate::service::demographic::user_write::ScimCommit;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{Change, WriteEnvelope};
use crate::versioning::lifecycle::state;

impl FerroEhrService {
    /// Writes `state` to the Group `id` of the client of `writer`, in one
    /// demographic CONTRIBUTION: a new version of its `GROUP` when its
    /// `displayName` or `externalId` changes, and a new version of each
    /// member's `PERSON` whose membership changes. Returns the Group as it
    /// stands after, or `None` when the id names no Group of that client.
    ///
    /// A write that changes nothing commits nothing. Only the members it
    /// adds are checked, so a Group keeps a member whose `PERSON` changed
    /// after it joined, and dropping a member whose deactivation already
    /// dropped it changes nothing.
    ///
    /// # Errors
    /// [`ServiceError::VersionConflict`] when the current version is not
    /// `expected`; a `422` naming `active` for a Group that is not active, and
    /// naming `members` for an added member that is no User of the client, is
    /// inactive or abandoned, or an add past `max_members`; the `409` of a
    /// `displayName` or `externalId` another Group holds; the commit refusals;
    /// a storage failure.
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
        let removed: Vec<VoId> = group
            .members
            .iter()
            .copied()
            .filter(|member| !members.contains(member))
            .collect();
        self.require_users(writer.client.binding_issuer, &added)
            .await?;
        let mut changes = Vec::new();
        let body = self
            .group_body(mapping, writer, Some(&group.body), &state)
            .await?;
        if body != group.body {
            let lifecycle = match self.party_template_fit(&body, mapping.template).await? {
                TemplateFit::Complete => state::COMPLETE,
                TemplateFit::Incomplete(_) => state::INCOMPLETE,
            };
            changes.push(Change::modify(
                id,
                Kind::Group,
                body,
                Some(self.group_tree(id).await?),
                None,
                WriteEnvelope {
                    lifecycle_state: Some(lifecycle.to_owned()),
                    ..WriteEnvelope::default()
                },
            ));
        }
        for member in added {
            changes.extend(self.membership_change(writer, id, member, true).await?);
        }
        for member in removed {
            changes.extend(self.membership_change(writer, id, member, false).await?);
        }
        if changes.is_empty() {
            return Ok(Some(group));
        }
        let commit = ScimCommit::new(writer.client.binding_issuer, change_type::MODIFICATION);
        self.commit_scim(commit, changes)
            .await
            .map_err(service_error)?;
        self.provisioned_group(writer.client, id).await
    }

    /// Logically deletes the Group `id` of the client of `writer`: its
    /// `GROUP`, with the membership each active member holds, in one
    /// demographic CONTRIBUTION (RM common
    /// `master06-change_control_package.adoc` §Logical Deletion). `false` when
    /// the id names no Group of that client.
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
        let mut changes = vec![Change::delete(
            id,
            Kind::Group,
            Some(self.group_tree(id).await?),
        )];
        for member in &group.members {
            changes.extend(self.membership_change(writer, id, *member, false).await?);
        }
        let commit = ScimCommit::new(writer.client.binding_issuer, change_type::DELETED);
        self.commit_scim(commit, changes)
            .await
            .map_err(service_error)?;
        Ok(true)
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
