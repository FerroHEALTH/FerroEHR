// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Users of a SCIM client as the demographic store holds them: each a
//! `PERSON` of a mapped template performing the professional `ROLE`, owned by
//! the client whose `binding_issuer` issues its binding key.
//!
//! A User is found through the binding side table
//! ([`crate::party_binding::provisioned`]) and served from its `PERSON`'s head
//! version. A creation commits the `PERSON` and its `ROLE` in one demographic
//! CONTRIBUTION, each naming the other (RM demographic
//! `org.openehr.rm.demographic.actor.adoc`, `…role.adoc`), under the
//! client's ownership ([`crate::party_binding::protect::provisioning`]), and
//! each version is `532|complete|` when its template holds in full and
//! `553|incomplete|` otherwise (RM common `master06-change_control_package.adoc`
//! §Incomplete Content). No openEHR spec governs SCIM: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use serde_json::Value;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::ids::VoId;
use crate::party_binding::provisioned::{self, UserFilter, UserRow};
use crate::service::FerroEhrService;
use crate::service::demographic::mapped_write::party_ref;
use crate::service::demographic::party_set::NewParty;
use crate::service::demographic::template::TemplateFit;
use crate::service::demographic::types::PartyKind;
use crate::storage::version_repo;
use crate::versioning::lifecycle;

/// A User of a SCIM client: its side-table row and its `PERSON`'s head.
#[derive(Debug, Clone)]
pub struct ProvisionedUser {
    /// The side-table row: the id, the binding key, the state, the
    /// `userName` and the `externalId`.
    pub row: UserRow,
    /// The head version's `OBJECT_VERSION_ID`, the User's version.
    pub version_uid: String,
    /// When the first version was committed.
    pub created: jiff::Timestamp,
    /// When the head version was committed.
    pub last_modified: jiff::Timestamp,
    /// The head version's canonical body.
    pub body: Value,
    /// The template paths an incomplete head lacks; empty otherwise.
    pub missing: Vec<String>,
}

/// One page of a client's Users.
#[derive(Debug, Clone)]
pub struct UserPage {
    /// How many Users match in all.
    pub total: u64,
    /// The Users of the page.
    pub users: Vec<ProvisionedUser>,
}

/// A User to create: the `PERSON`'s template, the namespace its references
/// are named under, and each mapped path with its data value.
#[derive(Debug, Clone)]
pub struct NewUser {
    /// The template of the `PERSON`, a mapped person template.
    pub template_id: String,
    /// The `PARTY_REF.namespace` the `PERSON` and `ROLE` name each other in.
    pub namespace: String,
    /// The data value of each mapped path, the selectors included.
    pub values: Vec<(String, Value)>,
}

impl FerroEhrService {
    /// The User `id` of the client of `binding_issuer`, or `None` when the id
    /// names no `PERSON` that client owns with a live professional `ROLE`.
    ///
    /// # Errors
    /// [`ServiceError`] on a storage failure.
    pub async fn provisioned_user(
        &self,
        binding_issuer: &str,
        id: VoId,
    ) -> Result<Option<ProvisionedUser>, ServiceError> {
        let filter = UserFilter {
            ids: vec![id.0],
            ..UserFilter::default()
        };
        let page = self
            .provisioned_users(binding_issuer, &filter, 0, 1)
            .await?;
        Ok(page.users.into_iter().next())
    }

    /// The Users of the client of `binding_issuer` matching `filter`, from
    /// the 0-based `offset`, at most `limit`, ordered by id.
    ///
    /// # Errors
    /// [`ServiceError`] on a storage failure, and an internal error when a
    /// User's `PERSON` cannot be read back.
    pub async fn provisioned_users(
        &self,
        binding_issuer: &str,
        filter: &UserFilter,
        offset: u64,
        limit: u64,
    ) -> Result<UserPage, ServiceError> {
        let page = (
            i64::try_from(offset).unwrap_or(i64::MAX),
            i64::try_from(limit).unwrap_or(i64::MAX),
        );
        let today = crate::party_binding::source::today();
        let (total, rows) = {
            let mut conn = acquire(&self.demographic_pool).await?;
            provisioned::users(&mut conn, binding_issuer, filter, &today, page).await?
        };
        let mut users = Vec::with_capacity(rows.len());
        for row in rows {
            users.push(self.user_of(row).await?);
        }
        Ok(UserPage {
            total: total.unsigned_abs(),
            users,
        })
    }

    /// Creates the User `user` for the client of `binding_issuer`: its
    /// `PERSON` and its professional `ROLE` of `role_template`, in one
    /// demographic CONTRIBUTION under the caller's committer.
    ///
    /// # Errors
    /// The [`Self::mapped_party_body`] and [`Self::professional_role_body`]
    /// refusals; the `422` of a body that does not fit its template even
    /// incomplete; the `409` of a binding key, `userName` or `externalId`
    /// another party holds; the `403` of a key the client's issuer does not
    /// issue; a storage failure.
    pub async fn provision_user(
        &self,
        binding_issuer: &str,
        role_template: &str,
        user: NewUser,
    ) -> Result<ProvisionedUser, ServiceError> {
        let (person, parties) = self.person_with_role(role_template, user, None).await?;
        Box::pin(crate::party_binding::protect::provisioning(
            binding_issuer.to_owned(),
            self.create_parties(parties, "SCIM provisioning: create User", None),
        ))
        .await?;
        self.provisioned_user(binding_issuer, person)
            .await?
            .ok_or_else(|| {
                ServiceError::exception(format!(
                    "the User {person} the SCIM client created is not found among its Users"
                ))
            })
    }

    /// The `PERSON` of `user` and a new professional `ROLE` of
    /// `role_template` it performs, each naming the other, ready for one
    /// CONTRIBUTION: the `PERSON`'s container and the two parties.
    ///
    /// The `PERSON` commits in `person_state`, or as [`Self::first_state`]
    /// judges it when `None`; the `ROLE` always as that judges it.
    pub(super) async fn person_with_role(
        &self,
        role_template: &str,
        user: NewUser,
        person_state: Option<&str>,
    ) -> Result<(VoId, Vec<NewParty>), ServiceError> {
        let (person_id, role_id) = (self.allocate_party_id(), self.allocate_party_id());
        let person = person_id.id();
        let mut body = self
            .mapped_party_body(None, &user.template_id, &user.values)
            .await?;
        if let Some(object) = body.as_object_mut() {
            object.insert(
                "roles".to_owned(),
                Value::Array(vec![party_ref(&user.namespace, "ROLE", role_id.id())]),
            );
        }
        let role = self
            .professional_role_body(role_template, person, &user.namespace)
            .await?;
        let person_state = match person_state {
            Some(state) => state.to_owned(),
            None => self.first_state(&body, &user.template_id).await?,
        };
        let parties = vec![
            NewParty {
                id: person_id,
                kind: PartyKind::Person,
                lifecycle_state: Some(person_state),
                body,
            },
            NewParty {
                id: role_id,
                kind: PartyKind::Role,
                lifecycle_state: Some(self.first_state(&role, role_template).await?),
                body: role,
            },
        ];
        Ok((person, parties))
    }

    /// The lifecycle state the first version of `body` commits in:
    /// `532|complete|` when it fits `template_id` in full, `553|incomplete|`
    /// when only the relaxed reading holds.
    pub(super) async fn first_state(
        &self,
        body: &Value,
        template_id: &str,
    ) -> Result<String, ServiceError> {
        Ok(match self.party_template_fit(body, template_id).await? {
            TemplateFit::Complete => lifecycle::state::COMPLETE,
            TemplateFit::Incomplete(_) => lifecycle::state::INCOMPLETE,
        }
        .to_owned())
    }

    /// The User the side-table row `row` names, read from its `PERSON`.
    async fn user_of(&self, row: UserRow) -> Result<ProvisionedUser, ServiceError> {
        let read = self
            .read_party(PartyKind::Person, row.party, None, None)
            .await?;
        let meta = read.meta.ok_or_else(|| {
            ServiceError::exception(format!("the PERSON {} has no current version", row.party))
        })?;
        let last_modified = meta.last_modified.ok_or_else(|| {
            ServiceError::exception(format!("the PERSON {} has no commit time", row.party))
        })?;
        let created = version_repo::meta::time_created(&self.demographic_pool, row.party)
            .await?
            .ok_or_else(|| {
                ServiceError::exception(format!("the PERSON {} has no first version", row.party))
            })?;
        let missing = if row.lifecycle_state == lifecycle::state::INCOMPLETE {
            self.missing_paths(&read.body).await?
        } else {
            Vec::new()
        };
        Ok(ProvisionedUser {
            row,
            version_uid: meta.uid,
            created,
            last_modified,
            body: read.body,
            missing,
        })
    }

    /// The paths the incomplete `PERSON` `body` lacks under its template.
    async fn missing_paths(&self, body: &Value) -> Result<Vec<String>, ServiceError> {
        let Some(template_id) = body
            .pointer("/archetype_details/template_id/value")
            .and_then(Value::as_str)
        else {
            return Ok(Vec::new());
        };
        Ok(match self.party_template_fit(body, template_id).await? {
            TemplateFit::Complete => Vec::new(),
            TemplateFit::Incomplete(violations) => {
                let mut paths: Vec<String> = violations
                    .iter()
                    .map(|violation| violation.path.clone())
                    .collect();
                paths.sort();
                paths.dedup();
                paths
            }
        })
    }
}
