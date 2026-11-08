// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Groups of a SCIM client as the demographic store holds them.
//!
//! Each is a `GROUP` of the mapped group template, with each member an inline
//! `PARTY_RELATIONSHIP` of the mapped membership archetype from the `GROUP`
//! to the member's `PERSON`.
//!
//! A Group's `displayName` is its `PARTY_IDENTITY` and its `externalId` a
//! `DV_IDENTIFIER` in `details` under the client's group issuer (RM
//! demographic `master02-demographic_package.adoc` §Names and Addresses,
//! §Party Identification). Its members are stored in its own version, since
//! "`PARTY_RELATIONSHIPs` are stored as part of the data of the `PARTY`
//! designated as the source" (§Party Relationships), and the direction
//! follows §Instance Examples, where the care team `GROUP` is the `source`;
//! the target is the member's `PERSON`, never a `ROLE`, since a membership
//! carries no credentials (`org.openehr.rm.demographic.role.adoc`). A Group
//! with no member has `relationships` Void (`party.adoc`
//! `Relationships_validity`). Every member is a User of the same client, so a
//! client can neither place another client's people in its Groups nor learn
//! that they exist, and a Group is never a member. No openEHR spec governs
//! SCIM: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use std::collections::BTreeSet;

use serde_json::Value;

use crate::db::acquire::acquire;
use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
use crate::party_binding::GroupMapping;
use crate::party_binding::provisioned::{self, GroupClient, GroupFilter, GroupRow};
use crate::service::FerroEhrService;
use crate::service::demographic::mapped_write::clear_mapped_value;
use crate::service::demographic::types::PartyKind;
use crate::service::demographic::user_write::{ScimCommit, UserWriteError};
use crate::storage::version_repo;
use crate::templates::identity::canonical_key;
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{Change, WriteEnvelope};

/// A Group of a SCIM client: its side-table row and its `GROUP`'s head.
#[derive(Debug, Clone)]
pub struct ProvisionedGroup {
    /// The side-table row: the id, the `displayName`, the state and the
    /// `externalId`.
    pub row: GroupRow,
    /// The head version's `OBJECT_VERSION_ID`, the Group's version.
    pub version_uid: String,
    /// When the first version was committed.
    pub created: jiff::Timestamp,
    /// When the head version was committed.
    pub last_modified: jiff::Timestamp,
    /// The members' `PERSON` containers, in the order the head holds them.
    pub members: Vec<VoId>,
    /// The head version's canonical body.
    pub body: Value,
}

/// One page of a client's Groups.
#[derive(Debug, Clone)]
pub struct GroupPage {
    /// How many Groups match in all.
    pub total: u64,
    /// The Groups of the page.
    pub groups: Vec<ProvisionedGroup>,
}

/// What a Group holds after a write: its `displayName`, its `externalId` and
/// its members.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupState {
    /// The `displayName`.
    pub display_name: String,
    /// The `externalId`, issued by the client (RFC 7643 §3.1).
    pub external_id: Option<String>,
    /// The members' `PERSON` containers.
    pub members: Vec<VoId>,
}

/// The SCIM client a Group write is of.
#[derive(Debug, Clone, Copy)]
pub struct GroupWriter<'a> {
    /// The client, by its `binding_issuer`, with its group issuer.
    pub client: GroupClient<'a>,
    /// The `PARTY_REF.namespace` the `GROUP` and its members are named in.
    pub namespace: &'a str,
    /// The most members a Group holds (`[scim] max_group_members`).
    pub max_members: usize,
}

impl FerroEhrService {
    /// The Group `id` of `client`, or `None` when the id names no live
    /// `GROUP` that client created.
    ///
    /// # Errors
    /// [`ServiceError`] on a storage failure.
    pub async fn provisioned_group(
        &self,
        client: GroupClient<'_>,
        id: VoId,
    ) -> Result<Option<ProvisionedGroup>, ServiceError> {
        let filter = GroupFilter {
            ids: vec![id.0],
            ..GroupFilter::default()
        };
        let page = self.provisioned_groups(client, &filter, 0, 1).await?;
        Ok(page.groups.into_iter().next())
    }

    /// The Groups of `client` matching `filter`, from the 0-based `offset`,
    /// at most `limit`, ordered by id.
    ///
    /// # Errors
    /// [`ServiceError`] on a storage failure, and an internal error when a
    /// Group's `GROUP` cannot be read back or the mapping names no group.
    pub async fn provisioned_groups(
        &self,
        client: GroupClient<'_>,
        filter: &GroupFilter,
        offset: u64,
        limit: u64,
    ) -> Result<GroupPage, ServiceError> {
        let page = (
            i64::try_from(offset).unwrap_or(i64::MAX),
            i64::try_from(limit).unwrap_or(i64::MAX),
        );
        let (total, rows) = {
            let mut conn = acquire(&self.demographic_pool).await?;
            provisioned::groups(&mut conn, client, filter, page).await?
        };
        let mut groups = Vec::with_capacity(rows.len());
        for row in rows {
            groups.push(self.group_of(row).await?);
        }
        Ok(GroupPage {
            total: total.unsigned_abs(),
            groups,
        })
    }

    /// Creates the Group `state` for the client of `writer`: one `GROUP` of
    /// the mapped group template holding a membership of each member, in one
    /// demographic CONTRIBUTION under the caller's committer, recorded as the
    /// client's.
    ///
    /// # Errors
    /// A `422` naming `members` when a member is no User of the client or
    /// the members exceed `max_members`; the `422` of a body that does not
    /// fit its template even incomplete; the `409` of a `displayName` or
    /// `externalId` another Group holds; a storage failure.
    pub async fn provision_group(
        &self,
        writer: GroupWriter<'_>,
        state: GroupState,
    ) -> Result<ProvisionedGroup, ServiceError> {
        let mapping = self.scim_group_mapping()?;
        let members = distinct(&state.members);
        if members.len() > writer.max_members {
            return Err(too_many(writer.max_members));
        }
        self.require_users(writer.client.binding_issuer, &members)
            .await?;
        let allocated = self.allocate_party_id();
        let id = allocated.id();
        let body = self
            .group_body(mapping, writer, None, (id, &state, &members))
            .await?;
        let lifecycle = self.first_state(&body, mapping.template).await?;
        let envelope = WriteEnvelope {
            lifecycle_state: Some(lifecycle),
            ..WriteEnvelope::default()
        };
        let commit = ScimCommit {
            created_group: Some(id),
            ..ScimCommit::new(writer.client.binding_issuer, change_type::CREATION)
        };
        self.commit_scim(
            commit,
            vec![Change::create_allocated(
                allocated,
                Kind::Group,
                body,
                envelope,
            )],
        )
        .await
        .map_err(service_error)?;
        self.provisioned_group(writer.client, id)
            .await?
            .ok_or_else(|| {
                ServiceError::exception(format!(
                    "the Group {id} the SCIM client created is not found among its Groups"
                ))
            })
    }

    /// The group mapping SCIM writes with.
    pub(super) fn scim_group_mapping(&self) -> Result<GroupMapping<'_>, ServiceError> {
        self.binding.group_mapping().ok_or_else(|| {
            ServiceError::exception(
                "the party mapping names no group template with its displayName and no \
                 membership archetype",
            )
        })
    }

    /// Refuses `members` unless each is a User of the client of
    /// `binding_issuer`: an unknown id, a Group and another client's User get
    /// the same answer.
    pub(super) async fn require_users(
        &self,
        binding_issuer: &str,
        members: &[VoId],
    ) -> Result<(), ServiceError> {
        if members.is_empty() {
            return Ok(());
        }
        let ids: Vec<uuid::Uuid> = members.iter().map(|member| member.0).collect();
        let today = crate::party_binding::source::today();
        let found: BTreeSet<uuid::Uuid> = {
            let mut conn = acquire(&self.demographic_pool).await?;
            provisioned::users_among(&mut conn, binding_issuer, &ids, &today)
                .await?
                .into_iter()
                .collect()
        };
        match ids.iter().find(|id| !found.contains(id)) {
            Some(id) => Err(ServiceError::content_invalid(
                Violation::new(format!(
                    "the member {id} is no User of this client: a member is a User the same \
                     client provisioned, never a Group"
                ))
                .with_path("members"),
            )),
            None => Ok(()),
        }
    }

    /// The body of the `GROUP` `id` holding `state` with its distinct
    /// `members`: `head` (a new `GROUP` when `None`) with the `displayName`
    /// and `externalId` written, and its memberships made `members`, each kept
    /// membership unchanged and every other relationship left as it is.
    pub(super) async fn group_body(
        &self,
        mapping: GroupMapping<'_>,
        writer: GroupWriter<'_>,
        head: Option<&Value>,
        (id, state, members): (VoId, &GroupState, &[VoId]),
    ) -> Result<Value, ServiceError> {
        let template = mapping.template;
        let (display, external) = (mapping.display_name.path(), mapping.external_id.path());
        let issued = (
            writer.client.group_issuer,
            mapping.external_id.identifier_type(),
        );
        let mut values = vec![(
            display.to_owned(),
            self.mapped_text_value(template, display, &state.display_name, issued)
                .await?,
        )];
        if let Some(external_id) = &state.external_id {
            values.push((
                external.to_owned(),
                self.mapped_text_value(template, external, external_id, issued)
                    .await?,
            ));
        }
        let mut body = self.mapped_party_body(head, template, &values).await?;
        if state.external_id.is_none() {
            clear_mapped_value(&mut body, external, Some((issued.1, issued.0)));
        }
        // Taken out of place without a copy; the attribute keeps its position.
        let stored = match body.get_mut("relationships") {
            Some(Value::Array(stored)) => std::mem::take(stored),
            _ => Vec::new(),
        };
        let wanted: BTreeSet<VoId> = members.iter().copied().collect();
        let mut present = BTreeSet::new();
        let mut relationships = Vec::with_capacity(stored.len().max(members.len()));
        for relationship in stored {
            if !is_membership(&relationship, mapping.membership) {
                relationships.push(relationship);
            } else if let Some(member) = target_of(&relationship)
                && wanted.contains(&member)
                && present.insert(member)
            {
                relationships.push(relationship);
            }
        }
        for member in members {
            if present.insert(*member) {
                relationships.push(
                    self.membership_body(mapping.membership, (id, *member), writer.namespace)
                        .await?,
                );
            }
        }
        if let Some(object) = body.as_object_mut() {
            // NOTE: party.adoc `Relationships_validity`, an empty list is invalid, so no member is Void.
            if relationships.is_empty() {
                object.remove("relationships");
            } else {
                object.insert("relationships".to_owned(), Value::Array(relationships));
            }
        }
        Ok(body)
    }

    /// The Group the side-table row `row` names, read from its `GROUP`.
    async fn group_of(&self, row: GroupRow) -> Result<ProvisionedGroup, ServiceError> {
        let mapping = self.scim_group_mapping()?;
        let read = self
            .read_party(PartyKind::Group, row.party, None, None)
            .await?;
        let meta = read.meta.ok_or_else(|| {
            ServiceError::exception(format!("the GROUP {} has no current version", row.party))
        })?;
        let last_modified = meta.last_modified.ok_or_else(|| {
            ServiceError::exception(format!("the GROUP {} has no commit time", row.party))
        })?;
        let created = version_repo::meta::time_created(&self.demographic_pool, row.party)
            .await?
            .ok_or_else(|| {
                ServiceError::exception(format!("the GROUP {} has no first version", row.party))
            })?;
        Ok(ProvisionedGroup {
            members: members_of(&read.body, mapping.membership),
            row,
            version_uid: meta.uid,
            created,
            last_modified,
            body: read.body,
        })
    }
}

/// The members of the `GROUP` `body`: the targets of its inline
/// relationships of the `membership` archetype, in the order it holds them.
#[must_use]
pub fn members_of(body: &Value, membership: &str) -> Vec<VoId> {
    let mut seen = BTreeSet::new();
    body.get("relationships")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|relationship| is_membership(relationship, membership))
        .filter_map(target_of)
        .filter(|member| seen.insert(*member))
        .collect()
}

/// Whether `relationship` is of the canonical `membership` archetype.
pub(super) fn is_membership(relationship: &Value, membership: &str) -> bool {
    relationship
        .get("archetype_node_id")
        .and_then(Value::as_str)
        .is_some_and(|id| canonical_key(id) == membership)
}

/// The container `relationship` targets.
pub(super) fn target_of(relationship: &Value) -> Option<VoId> {
    // NOTE: a reference whose root is no UUID names no container of this store, so no member.
    relationship
        .pointer("/target/id/value")
        .and_then(Value::as_str)
        .and_then(|value| VoId::from_uid_head(value).ok())
}

/// `members` without repeats, in their first order.
pub(super) fn distinct(members: &[VoId]) -> Vec<VoId> {
    let mut seen = BTreeSet::new();
    members
        .iter()
        .copied()
        .filter(|member| seen.insert(*member))
        .collect()
}

/// The refusal of a Group past `max` members.
pub(super) fn too_many(max: usize) -> ServiceError {
    ServiceError::content_invalid(
        Violation::new(format!(
            "a Group holds at most {max} members ([scim] max_group_members)"
        ))
        .with_path("members"),
    )
}

/// The service error a SCIM commit of a Group fails with: a Group write
/// counts against no deactivation bound.
pub(super) fn service_error(error: UserWriteError) -> ServiceError {
    match error {
        UserWriteError::Service(error) => error,
        limited @ UserWriteError::Limited { .. } => ServiceError::exception(limited.to_string()),
    }
}
